//! An MCP server living inside the running editor.
//!
//! The point is to let an agent see and drive the editor that is already open,
//! rather than reason about the project from its files. A file on disk does not
//! say which entities exist after a script ran, what the console printed, or
//! what the viewport currently looks like, and those are exactly the questions
//! worth asking mid-session.
//!
//! Three pieces, in the order a request passes through them:
//!
//! - [`http`] owns a listener thread. It speaks just enough HTTP/1.1 to answer a
//!   local client and hands the body to [`rpc`].
//! - [`rpc`] is the MCP envelope: `initialize`, `tools/list`, `tools/call`. A
//!   call becomes a [`ToolCall`] posted down a channel, and the calling thread
//!   then blocks on its own reply channel.
//! - [`tools`] drains that channel from an exclusive system, so every tool runs
//!   on the main thread with `&mut World` and none of them has to reason about
//!   what else is touching the world at the time.
//!
//! That split is the whole design. Bevy's world is not `Sync` and a socket
//! cannot wait for a frame, so the two halves have to be different threads with
//! a queue between them, and the queue is what makes the tools trivial to write.
//!
//! Edits go on the editor's own undo stack through the undo core in the contract
//! crate (see [`edits`]), so Ctrl+Z takes back what an agent did. The exceptions
//! are despawning and removing a component, which the tool list marks
//! destructive.

use bevy::prelude::*;
use renzora::core::console_log::{console_error, console_info};
use renzora::{RenzoraShellExt, ShellStatusAlign, ShellStatusItem, ShellStatusSegment};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

mod b64;
mod content;
mod edits;
mod http;
mod rpc;
mod tools;

/// The console category every line from this plugin carries, so the Console's
/// filter chips get one stable name rather than one per module.
pub const CATEGORY: &str = "mcp";

/// The port the server binds unless `RENZORA_MCP_PORT` says otherwise.
///
/// Fixed rather than "first free port above N" on purpose: the URL is written
/// once into an MCP client's config and never looked at again, so a port that
/// moves when something else happens to hold 47800 would silently point that
/// config at nothing. Failing to bind is louder and easier to act on than
/// succeeding somewhere the client is not looking.
pub const DEFAULT_PORT: u16 = 47800;

/// What the server is doing, as seen from the main thread.
///
/// Three `u8` states behind an atomic rather than a `Mutex<enum>`: the status
/// bar reads this every frame from `&World`, and a frame should never be able to
/// block on a lock held by a socket thread.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Binding,
    Listening,
    Failed,
}

/// State shared between the listener thread and the status bar.
pub struct ServerState {
    pub port: u16,
    phase: AtomicU8,
    /// Connections currently open. An MCP client holds one for the life of the
    /// session, so "more than zero" is the honest reading of "a client is
    /// attached" without inventing a session concept the server does not have.
    live: AtomicU64,
    calls: AtomicU64,
    /// Why the bind failed, shown in the status bar's tooltip position. Only
    /// written once, on the way into [`Phase::Failed`].
    detail: Mutex<String>,
    /// When the pump last ran, in milliseconds since `started`; zero until the
    /// first frame. This is how a socket thread tells a frozen editor from a busy
    /// one without waiting out the whole call timeout to find out.
    last_pump_ms: AtomicU64,
    started: Instant,
}

impl ServerState {
    fn new(port: u16) -> Self {
        Self {
            port,
            phase: AtomicU8::new(0),
            live: AtomicU64::new(0),
            calls: AtomicU64::new(0),
            detail: Mutex::new(String::new()),
            last_pump_ms: AtomicU64::new(0),
            started: Instant::now(),
        }
    }

    /// Called by the pump every frame.
    pub fn mark_pump(&self) {
        // `max(1)` so a pump in the first millisecond is not read as "never".
        let ms = self.started.elapsed().as_millis().max(1) as u64;
        self.last_pump_ms.store(ms, Ordering::Relaxed);
    }

    /// How long since the main thread last ran a frame, or `None` before the
    /// first one.
    pub fn since_pump(&self) -> Option<std::time::Duration> {
        let last = self.last_pump_ms.load(Ordering::Relaxed);
        (last != 0).then(|| {
            let now = self.started.elapsed().as_millis() as u64;
            std::time::Duration::from_millis(now.saturating_sub(last))
        })
    }

    pub fn phase(&self) -> Phase {
        match self.phase.load(Ordering::Relaxed) {
            1 => Phase::Listening,
            2 => Phase::Failed,
            _ => Phase::Binding,
        }
    }

    pub fn set_listening(&self) {
        self.phase.store(1, Ordering::Relaxed);
    }

    pub fn set_failed(&self, why: impl Into<String>) {
        if let Ok(mut d) = self.detail.lock() {
            *d = why.into();
        }
        self.phase.store(2, Ordering::Relaxed);
    }

    pub fn detail(&self) -> String {
        self.detail.lock().map(|d| d.clone()).unwrap_or_default()
    }

    pub fn client_opened(&self) {
        self.live.fetch_add(1, Ordering::Relaxed);
    }

    pub fn client_closed(&self) {
        // `fetch_sub` on an unsigned counter wraps rather than saturating, and a
        // close without a matching open would park the status bar on "4 clients"
        // for the rest of the session.
        let _ = self
            .live
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_sub(1))
            });
    }

    pub fn clients(&self) -> u64 {
        self.live.load(Ordering::Relaxed)
    }

    pub fn call_made(&self) {
        self.calls.fetch_add(1, Ordering::Relaxed);
    }

    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
}

/// One `tools/call`, on its way from a socket thread to the main thread.
pub struct ToolCall {
    pub name: String,
    pub args: renzora::serde_json::Value,
    /// Where the answer goes. A channel per call rather than one shared return
    /// queue, because two clients (or one client with two in-flight calls) would
    /// otherwise have to agree on whose answer they are reading.
    pub reply: Sender<ToolReply>,
    /// When the caller stops waiting. A call still queued after this is dropped
    /// unrun: the client has already been told it failed, and may well have
    /// retried, so running it late would do the thing twice.
    pub expires: Instant,
}

/// What a tool produced. The three variants are exactly MCP's three outcomes for
/// a call, so [`rpc`] can turn one into a response without a second decision.
pub enum ToolReply {
    Text(String),
    Image { data: String, mime: &'static str },
    Failed(String),
}

/// The main-thread end of the queue, plus the state the status bar reads.
///
/// `Receiver` is `Send` but not `Sync`, and a Bevy resource must be both, so it
/// sits behind a `Mutex` that only the pump ever locks.
#[derive(Resource)]
pub struct McpBridge {
    pub rx: Mutex<Receiver<ToolCall>>,
    pub state: Arc<ServerState>,
}

/// A screenshot that has been asked for and not yet landed on disk.
///
/// Screenshots are the one tool that cannot answer within the frame that started
/// it: the capture is resolved by the render app a frame or more later, so the
/// reply channel has to outlive the call. Everything else replies before the
/// pump returns.
pub struct PendingShot {
    pub path: PathBuf,
    pub reply: Sender<ToolReply>,
    pub deadline: Instant,
}

#[derive(Resource, Default)]
pub struct PendingShots(pub Vec<PendingShot>);

pub struct McpPlugin;

impl Plugin for McpPlugin {
    fn build(&self, app: &mut App) {
        let port = std::env::var("RENZORA_MCP_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);

        let state = Arc::new(ServerState::new(port));
        let (tx, rx) = std::sync::mpsc::channel::<ToolCall>();

        http::spawn(Arc::clone(&state), tx);

        app.insert_resource(McpBridge {
            rx: Mutex::new(rx),
            state,
        })
        .init_resource::<PendingShots>()
        .init_resource::<tools::AssetCache>()
        .init_resource::<content::PendingImports>()
        .add_systems(Update, (tools::pump, collect_screenshots, content::collect_imports))
        .add_systems(Startup, announce);

        app.register_shell_status_item(ShellStatusItem {
            id: "mcp",
            align: ShellStatusAlign::Right,
            // To the left of the system monitor's readouts, which sit at 0. A
            // connection indicator that moves when the FPS counter changes width
            // is hard to find twice.
            order: -1,
            render: status_segments,
        });
    }
}

/// Print the one line somebody needs to point a client at this.
///
/// In the console rather than only in a doc, because the port is configurable:
/// the command that works is the one built from the port actually bound, not the
/// one written down when the README was.
fn announce(bridge: Res<McpBridge>) {
    let port = bridge.state.port;
    console_info(
        CATEGORY,
        format!("MCP server on http://127.0.0.1:{port}/mcp"),
    );
    console_info(
        CATEGORY,
        format!("claude mcp add --transport http renzora http://127.0.0.1:{port}/mcp"),
    );
}

/// Move finished screenshots from disk into their waiting reply channel.
///
/// Polling a file rather than observing `ScreenshotCaptured` directly is what
/// lets this plugin encode a PNG without linking the `image` crate: Bevy's own
/// `save_to_disk` observer already does that encode, so the cheapest correct
/// path is to let it write the file and pick the bytes up here.
fn collect_screenshots(mut pending: ResMut<PendingShots>) {
    if pending.0.is_empty() {
        return;
    }

    let now = Instant::now();
    pending.0.retain(|shot| {
        if let Some(bytes) = tools::read_complete_png(&shot.path) {
            let _ = shot.reply.send(ToolReply::Image {
                data: b64::encode(&bytes),
                mime: "image/png",
            });
            let _ = std::fs::remove_file(&shot.path);
            return false;
        }

        if now >= shot.deadline {
            let _ = shot
                .reply
                .send(ToolReply::Failed("the screenshot never arrived".into()));
            let _ = std::fs::remove_file(&shot.path);
            return false;
        }

        true
    });
}

/// The status-bar segment: one icon plus the port, tinted by what the server is
/// doing. A failed bind says so rather than showing nothing, because "no MCP
/// segment at all" and "the plugin did not load" look identical.
fn status_segments(world: &World) -> Vec<ShellStatusSegment> {
    let Some(bridge) = world.get_resource::<McpBridge>() else {
        return Vec::new();
    };
    let state = &bridge.state;

    const IDLE: [u8; 3] = [150, 150, 164];
    const LIVE: [u8; 3] = [100, 200, 120];
    const BAD: [u8; 3] = [220, 80, 80];

    let (icon, text, color) = match state.phase() {
        Phase::Binding => ("plugs", format!("MCP {}", state.port), IDLE),
        Phase::Failed => (
            "network-slash",
            format!("MCP {}", state.detail()),
            BAD,
        ),
        Phase::Listening if state.clients() > 0 => (
            "plugs-connected",
            format!("MCP {} calls", state.calls()),
            LIVE,
        ),
        Phase::Listening => ("plugs", format!("MCP {}", state.port), IDLE),
    };

    vec![ShellStatusSegment::new(icon, text, color)]
}

/// Report a bind failure once, from the thread that hit it.
pub(crate) fn report_bind_failure(state: &ServerState, err: &std::io::Error) {
    let port = state.port;
    state.set_failed(format!("port {port} busy"));
    console_error(
        CATEGORY,
        format!("MCP server could not bind 127.0.0.1:{port}: {err}"),
    );
}

// `plugin!` with the default `Editor` scope. A shipped game has no editor to
// drive and no business opening a local port that accepts commands, so this must
// not be `Runtime`.
renzora::plugin!(McpPlugin);
