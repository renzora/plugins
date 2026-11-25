//! The socket Blender connects to, and the framing on it.
//!
//! One listener thread accepts connections; each connection gets a reader
//! thread that turns bytes into [`Incoming`] messages on a channel the main
//! thread drains. The world is never touched from here: Bevy's world is not
//! `Sync` and a socket cannot wait for a frame, so the two halves live on
//! different threads with a queue between them, as in the MCP plugin.
//!
//! ## Wire format
//!
//! Every frame is
//!
//! ```text
//! u32 LE  header length
//! u32 LE  payload length
//! header  UTF-8 JSON object with a "type" field
//! payload raw bytes the header describes (vertex arrays, image files)
//! ```
//!
//! JSON for the small, irregular part and raw little-endian arrays for the
//! large, regular part. A mesh of a million triangles would be hundreds of
//! megabytes of JSON numbers and seconds of parsing; as arrays it is a copy.

use renzora::serde_json::{self, Value};
use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// Headers are small JSON objects; anything this large is not one of ours.
const MAX_HEADER: usize = 16 * 1024 * 1024;
/// A generous ceiling on one payload (a very dense mesh, or a big texture), so
/// a stray connection speaking another protocol cannot make us allocate
/// whatever its first eight bytes happen to say.
const MAX_PAYLOAD: usize = 1024 * 1024 * 1024;

/// What a connection thread hands the main thread.
pub enum Incoming {
    /// A Blender connected. `conn` numbers connections so the main thread can
    /// ignore stragglers from one it has already replaced.
    Connected { conn: u64, writer: TcpStream },
    Message { conn: u64, header: Value, payload: Vec<u8> },
    Closed { conn: u64, why: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Binding,
    Listening,
    Failed,
}

/// What the status bar reads. Atomics, so a frame never waits on a socket
/// thread's lock.
pub struct ServerState {
    pub port: u16,
    phase: AtomicU8,
    connected: AtomicU8,
    next_conn: AtomicU64,
    detail: Mutex<String>,
}

impl ServerState {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            phase: AtomicU8::new(0),
            connected: AtomicU8::new(0),
            next_conn: AtomicU64::new(1),
            detail: Mutex::new(String::new()),
        }
    }

    pub fn phase(&self) -> Phase {
        match self.phase.load(Ordering::Relaxed) {
            1 => Phase::Listening,
            2 => Phase::Failed,
            _ => Phase::Binding,
        }
    }

    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed) > 0
    }

    pub fn detail(&self) -> String {
        self.detail.lock().map(|d| d.clone()).unwrap_or_default()
    }

    fn set_failed(&self, why: String) {
        if let Ok(mut d) = self.detail.lock() {
            *d = why;
        }
        self.phase.store(2, Ordering::Relaxed);
    }
}

/// Start listening on `127.0.0.1:<port>`.
///
/// Loopback only: the link takes whatever geometry it is sent and writes files
/// into the project, which is not something to offer the local network.
pub fn spawn(state: Arc<ServerState>, tx: Sender<Incoming>) {
    let spawned = std::thread::Builder::new()
        .name("blender-link-listen".into())
        .spawn(move || {
            let listener = match TcpListener::bind(("127.0.0.1", state.port)) {
                Ok(l) => l,
                Err(e) => {
                    state.set_failed(format!("port {} busy ({e})", state.port));
                    return;
                }
            };
            state.phase.store(1, Ordering::Relaxed);
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let _ = stream.set_nodelay(true);
                let conn = state.next_conn.fetch_add(1, Ordering::Relaxed);
                let Ok(writer) = stream.try_clone() else { continue };
                if tx.send(Incoming::Connected { conn, writer }).is_err() {
                    return;
                }
                let tx = tx.clone();
                let state = Arc::clone(&state);
                let _ = std::thread::Builder::new()
                    .name(format!("blender-link-{conn}"))
                    .spawn(move || {
                        state.connected.fetch_add(1, Ordering::Relaxed);
                        let why = read_frames(stream, conn, &tx);
                        let _ = state.connected.fetch_update(
                            Ordering::Relaxed,
                            Ordering::Relaxed,
                            |n| Some(n.saturating_sub(1)),
                        );
                        let _ = tx.send(Incoming::Closed { conn, why });
                    });
            }
        });
    if let Err(e) = spawned {
        bevy::log::error!("[blender_link] could not start the listener thread: {e}");
    }
}

/// Read frames until the connection ends; returns why it ended.
fn read_frames(stream: TcpStream, conn: u64, tx: &Sender<Incoming>) -> String {
    let mut reader = BufReader::with_capacity(1 << 20, stream);
    loop {
        let mut lens = [0u8; 8];
        if let Err(e) = reader.read_exact(&mut lens) {
            return if e.kind() == std::io::ErrorKind::UnexpectedEof {
                "Blender disconnected".into()
            } else {
                format!("connection lost ({e})")
            };
        }
        let header_len = u32::from_le_bytes([lens[0], lens[1], lens[2], lens[3]]) as usize;
        let payload_len = u32::from_le_bytes([lens[4], lens[5], lens[6], lens[7]]) as usize;
        if header_len > MAX_HEADER || payload_len > MAX_PAYLOAD {
            return format!(
                "a frame claimed a {header_len} byte header and a {payload_len} byte payload; this is not the Blender add-on talking"
            );
        }
        let mut header = vec![0u8; header_len];
        let mut payload = vec![0u8; payload_len];
        if let Err(e) = reader
            .read_exact(&mut header)
            .and_then(|_| reader.read_exact(&mut payload))
        {
            return format!("connection lost mid-frame ({e})");
        }
        let header: Value = match serde_json::from_slice(&header) {
            Ok(v) => v,
            Err(e) => return format!("a frame header was not JSON ({e})"),
        };
        if tx.send(Incoming::Message { conn, header, payload }).is_err() {
            return "the editor is shutting down".into();
        }
    }
}

/// Send one frame with no payload. Small and rare (the welcome), so it is
/// written from the main thread directly.
pub fn send(writer: &mut TcpStream, header: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(header).unwrap_or_default();
    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes());
    frame.extend_from_slice(&body);
    writer.write_all(&frame)
}
