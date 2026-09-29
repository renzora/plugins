//! The listener thread, and just enough HTTP/1.1 to answer one local client.
//!
//! This is MCP's "streamable HTTP" transport reduced to the half a stateless
//! server needs: the client POSTs a JSON-RPC message to `/mcp` and reads the
//! reply from the same response body. The other half of that transport, a GET
//! that stays open as a server-sent-event stream so the server can speak first,
//! is answered with 405. The spec allows exactly that, and nothing here has
//! anything to say unprompted: every message this server sends is a reply.
//!
//! Hand-written rather than a crate, for the reason set out in `Cargo.toml`. The
//! surface that buys is small: one verb, one path, `Content-Length` bodies, and
//! keep-alive by doing nothing, since HTTP/1.1 connections persist unless a
//! `Connection: close` says otherwise.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;

use crate::{report_bind_failure, rpc, ServerState, ToolCall};

/// Connections served at once before new ones are turned away.
///
/// A client holds one connection for the life of its session, so this is not a
/// throughput limit, it is a bound on how many threads a misbehaving or
/// reconnecting client can leave parked in a blocking read.
const MAX_LIVE_CONNECTIONS: u64 = 32;

/// Largest request body accepted, in bytes.
///
/// Arguments are small JSON objects. The cap exists so a bad `Content-Length`
/// cannot make the server allocate whatever number it was sent.
const MAX_BODY: usize = 1 << 20;

/// Start listening. Never blocks the caller: the whole server lives on threads
/// this spawns, because `build` is called while the `App` is being assembled.
pub fn spawn(state: Arc<ServerState>, tx: Sender<ToolCall>) {
    let listener_state = Arc::clone(&state);
    let spawned = thread::Builder::new()
        .name("renzora-mcp".into())
        .spawn(move || {
            // Localhost only, and not by accident. These tools spawn entities,
            // save scenes and read the console of whatever project is open, so
            // the server has no business being reachable from the network.
            let listener = match TcpListener::bind(("127.0.0.1", listener_state.port)) {
                Ok(listener) => listener,
                Err(err) => {
                    report_bind_failure(&listener_state, &err);
                    return;
                }
            };
            listener_state.set_listening();

            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                if listener_state.clients() >= MAX_LIVE_CONNECTIONS {
                    let mut stream = stream;
                    respond(&mut stream, "503 Service Unavailable", None, b"");
                    continue;
                }

                let state = Arc::clone(&listener_state);
                let tx = tx.clone();
                let conn = thread::Builder::new()
                    .name("renzora-mcp-conn".into())
                    .spawn(move || {
                        state.client_opened();
                        serve(stream, &state, &tx);
                        state.client_closed();
                    });
                // A connection thread that cannot start is dropped rather than
                // retried: the client will reconnect, and a busy-loop of failing
                // spawns would be worse than the dropped request.
                drop(conn);
            }
        });
    drop(spawned);
}

/// Serve one connection until the peer closes it or sends something unparseable.
fn serve(stream: TcpStream, state: &ServerState, tx: &Sender<ToolCall>) {
    // Small JSON payloads land in one segment each way, so Nagle's algorithm has
    // nothing to coalesce and only adds latency to every call.
    let _ = stream.set_nodelay(true);

    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_half);
    let mut writer = stream;

    loop {
        let Some(request) = read_request(&mut reader) else {
            return;
        };

        let path = request.path.as_str();
        let is_mcp = path == "/mcp" || path == "/";

        if let Err(why) = check_caller(&request, state.port) {
            respond(&mut writer, "403 Forbidden", Some("text/plain"), why.as_bytes());
            if request.close {
                return;
            }
            continue;
        }

        match request.method.as_str() {
            "POST" if is_mcp => match rpc::handle(&request.body, tx, state) {
                // A JSON-RPC notification has no reply. 202 with an empty body is
                // what the transport expects there; answering `{}` instead makes
                // a strict client complain about a response it never asked for.
                None => respond(&mut writer, "202 Accepted", None, b""),
                Some(body) => respond(
                    &mut writer,
                    "200 OK",
                    Some("application/json"),
                    body.as_bytes(),
                ),
            },
            // The client ending its session. There is no session state to drop,
            // so this only has to be polite about it.
            "DELETE" if is_mcp => respond(&mut writer, "200 OK", None, b""),
            "GET" if is_mcp => respond(
                &mut writer,
                "405 Method Not Allowed",
                Some("text/plain"),
                b"this server replies to POST only; it opens no SSE stream\n",
            ),
            _ => respond(&mut writer, "404 Not Found", None, b""),
        }

        if request.close {
            return;
        }
    }
}

/// Refuse anything a web browser could have sent.
///
/// Binding to 127.0.0.1 keeps the network out, but it does not keep out a web
/// page open on this same machine. A page can POST to `http://127.0.0.1:47800`
/// with a `text/plain` body, which a browser sends without asking the server
/// first, and every tool here would run: despawn, save, export. DNS rebinding goes
/// further and gives an attacker's own hostname this address. So three checks,
/// each of which a browser cannot get past and a local MCP client never trips:
///
/// - `Host` must name loopback. A rebound request carries the attacker's hostname.
/// - `Origin`, when present, must be loopback. CLI clients send none; a browser
///   always sends one on a cross-origin POST.
/// - A POST must be `application/json`. A browser only sends that type after a
///   CORS preflight, and this server answers no preflight, so it never follows.
///
/// A token would also work, but it would turn the one-line `claude mcp add` into
/// a copy of a secret that changes, and these checks already stop the browser,
/// which is the only caller that is not the user.
fn check_caller(request: &Request, port: u16) -> Result<(), String> {
    let loopback = |authority: &str| {
        let (host, port_part) = split_authority(authority);
        matches!(host, "127.0.0.1" | "localhost" | "::1")
            && port_part.is_none_or(|p| p == port.to_string())
    };

    match request.host.as_deref() {
        Some(host) if loopback(host) => {}
        Some(host) => return Err(format!("refused: Host {host} is not this machine\n")),
        None => return Err("refused: no Host header\n".into()),
    }

    if let Some(origin) = request.origin.as_deref() {
        let authority = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
            .unwrap_or("");
        if !loopback(authority) {
            return Err(format!("refused: requests from {origin} are not accepted\n"));
        }
    }

    if request.method == "POST" {
        let json = request
            .content_type
            .as_deref()
            .and_then(|ct| ct.split(';').next())
            .is_some_and(|ct| ct.trim().eq_ignore_ascii_case("application/json"));
        if !json {
            return Err("refused: a POST must be Content-Type: application/json\n".into());
        }
    }

    Ok(())
}

/// `host[:port]` into its parts. An IPv6 host is bracketed, `[::1]:47800`, because
/// its own colons would otherwise be read as the port separator.
fn split_authority(authority: &str) -> (&str, Option<&str>) {
    if let Some(rest) = authority.strip_prefix('[') {
        let Some((host, after)) = rest.split_once(']') else {
            return (authority, None);
        };
        return (host, after.strip_prefix(':'));
    }
    match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    }
}

struct Request {
    method: String,
    path: String,
    body: String,
    /// The peer asked for the connection to end after this exchange.
    close: bool,
    host: Option<String>,
    origin: Option<String>,
    content_type: Option<String>,
}

/// Read one request, or `None` at end of stream or on anything malformed.
///
/// Malformed and closed are deliberately the same answer. This server has one
/// client and no way to resynchronise a broken stream, so the useful response to
/// a request it cannot parse is to stop reading that connection.
fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }

    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut content_length = 0usize;
    let mut close = false;
    let mut host = None;
    let mut origin = None;
    let mut content_type = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
            return None;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let value = value.trim();
        // Header names are case-insensitive, and clients disagree about how to
        // spell them.
        match name.to_ascii_lowercase().as_str() {
            "content-length" => content_length = value.parse().ok()?,
            "connection" => close = value.eq_ignore_ascii_case("close"),
            "host" => host = Some(value.to_string()),
            "origin" => origin = Some(value.to_string()),
            "content-type" => content_type = Some(value.to_string()),
            _ => {}
        }
    }

    if content_length > MAX_BODY {
        return None;
    }

    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).ok()?;

    Some(Request {
        method,
        path,
        body: String::from_utf8(body).ok()?,
        close,
        host,
        origin,
        content_type,
    })
}

/// Write one response. Always sends `Content-Length`, so the connection stays
/// usable for the next request rather than having to be closed to delimit this
/// one.
fn respond(stream: &mut TcpStream, status: &str, content_type: Option<&str>, body: &[u8]) {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
    if let Some(ct) = content_type {
        head.push_str(&format!("Content-Type: {ct}\r\n"));
    }
    head.push_str("\r\n");

    // A write that fails means the peer is gone. The caller finds that out on
    // its next read, and there is nowhere useful to report it from here.
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}
