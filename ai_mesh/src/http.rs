//! The transport every provider shares: JSON in and out, binary downloads, the
//! encodings an image has to be wrapped in, and turning a provider's error body
//! into a sentence.
//!
//! # Why every function here blocks
//!
//! [`renzora::net::Request::send`] parks *its own thread* until the per-frame
//! pump delivers the answer. Called from a system it waits for a frame that
//! cannot advance until the system returns, which the contract crate reports as
//! `Error::NoPump`. So these run only on the worker thread a job owns, never on
//! the main thread. See [`crate::job::spawn`].

use renzora::net::Request;
use renzora::serde_json::Value;

/// Sent on every request. A literal, not `env!("CARGO_PKG_VERSION")`: a native
/// plugin is compiled by a bare `rustc`, and the only cargo variables set for it
/// are `CARGO_MANIFEST_DIR` and `CARGO_PKG_NAME`.
pub const USER_AGENT: &str = "renzora-ai-mesh/0.1";

/// Cap on one generated model. The largest outputs (Rodin-class sculpts, 8k PBR
/// textures) are a few hundred MB; anything past this is not a game asset.
const MAX_MODEL_BYTES: u32 = 512 * 1024 * 1024;

/// Cap on a JSON response. Status bodies are a few KB, but the local Hunyuan3D
/// server returns the finished model *inside* its status JSON as base64, so this
/// has to clear a model too.
const MAX_JSON_BYTES: u32 = 768 * 1024 * 1024;

/// How an API wants its key presented.
#[derive(Clone, Copy)]
pub enum Auth<'a> {
    /// `Authorization: Bearer <key>`: Meshy and Tripo.
    Bearer(&'a str),
    /// `Authorization: Key <key>`: fal.ai.
    FalKey(&'a str),
    /// No credential: a local server, or a signed download URL.
    None,
}

fn authorize(request: Request, auth: Auth) -> Request {
    let request = request.header("User-Agent", USER_AGENT);
    match auth {
        Auth::Bearer(key) => request.bearer(key),
        Auth::FalKey(key) => request.header("Authorization", &format!("Key {key}")),
        Auth::None => request,
    }
}

/// POST a JSON body and parse the JSON reply.
pub fn post_json(url: &str, auth: Auth, body: &Value) -> Result<Value, String> {
    let request = authorize(Request::post(url), auth)
        .json(body)
        .max_bytes(MAX_JSON_BYTES);
    send_json(request, url)
}

/// POST a prebuilt body (a multipart upload) and parse the JSON reply.
pub fn post_raw(url: &str, auth: Auth, content_type: &str, body: Vec<u8>) -> Result<Value, String> {
    let request = authorize(Request::post(url), auth)
        .body(content_type, body)
        .max_bytes(MAX_JSON_BYTES);
    send_json(request, url)
}

/// GET a JSON document.
pub fn get_json(url: &str, auth: Auth) -> Result<Value, String> {
    let request = authorize(Request::get(url), auth).max_bytes(MAX_JSON_BYTES);
    send_json(request, url)
}

/// GET a file, as bytes.
///
/// Download URLs are signed (Meshy, Tripo, fal's CDN), so no auth header goes
/// with them: some storage backends reject a request that carries a credential
/// they did not issue.
pub fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let response = authorize(Request::get(url), Auth::None)
        .max_bytes(MAX_MODEL_BYTES)
        .send()
        .map_err(|e| e.to_string())?;
    if !response.is_ok() {
        return Err(format!("download failed: HTTP {}", response.status));
    }
    Ok(response.body)
}

/// Send a request whose answer does not matter: the remote half of a cancel.
///
/// Errors are dropped on purpose. The job is already over on this side, and a
/// cancel the provider refused (Meshy answers `409` once a task is running) is
/// not something the user can act on.
pub fn fire(method: &str, url: &str, auth: Auth) {
    let _ = authorize(Request::new(method, url), auth).send();
}

fn send_json(request: Request, url: &str) -> Result<Value, String> {
    let response = request.send().map_err(|e| e.to_string())?;
    // Read the body ourselves rather than through `Response::json`: that one
    // only recognises `{"error": ...}`, and none of these providers use it alone.
    // Meshy says `message`, Tripo says `message` plus `suggestion`, fal says
    // `detail`. Reporting "HTTP 402" and dropping "not enough credits" is exactly
    // the failure that makes a paid API feel broken.
    let text = response.text();
    if !response.is_ok() {
        return Err(error_message(response.status, &text));
    }
    renzora::serde_json::from_str(&text)
        .map_err(|e| format!("{url} did not return JSON: {e}"))
}

/// The most useful sentence a failed request's body holds.
pub fn error_message(status: u16, body: &str) -> String {
    let parsed: Option<Value> = renzora::serde_json::from_str(body).ok();
    let found = parsed.as_ref().and_then(|v| {
        let message = ["message", "error", "detail"]
            .iter()
            .find_map(|key| text_of(v.get(*key)?))?;
        match v.get("suggestion").and_then(Value::as_str) {
            Some(hint) if !hint.is_empty() => Some(format!("{message} ({hint})")),
            _ => Some(message),
        }
    });
    let hint = match status {
        401 | 403 => " Check the API key in Settings > Plugins > AI Mesh.",
        402 => " The account is out of credits.",
        429 => " Too many requests; wait and try again.",
        _ => "",
    };
    match found {
        Some(message) => format!("{message}{hint}"),
        None => format!("HTTP {status}.{hint}"),
    }
}

/// A field as a sentence, whether the provider sent a string or a structure.
///
/// fal's `detail` is a string on a 401 and a list of validation objects on a 422,
/// so a string-only reader would show nothing for the error a user is most
/// likely to hit while trying options.
fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|item| text_of(item.get("msg").unwrap_or(item)))
                .collect();
            (!parts.is_empty()).then(|| parts.join("; "))
        }
        Value::Object(_) => value.get("message").and_then(text_of),
        _ => None,
    }
}

// ============================================================================
// Encodings
// ============================================================================

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64, padded.
///
/// Written out rather than taken from the `base64` crate: it is the only thing
/// this plugin would need a crate for besides the file dialog, and every crate
/// cargo has to resolve is a network round trip on the user's first build.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = match chunk.len() {
            3 => (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8 | chunk[2] as u32,
            2 => (chunk[0] as u32) << 16 | (chunk[1] as u32) << 8,
            _ => (chunk[0] as u32) << 16,
        };
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Decode standard base64, ignoring whitespace and padding.
pub fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\r' | b'\n' | b' ' => continue,
            _ => return Err("the server returned a model that is not valid base64".into()),
        };
        // Masked so only the bits not yet emitted are kept; at most 14 are ever
        // live, and the mask makes that a property rather than a reading.
        acc = ((acc << 6) | value as u32) & 0xFFFF;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// The MIME type of an image the providers accept, from its extension.
pub fn image_mime(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// `data:<mime>;base64,<...>`, which Meshy and fal both accept in place of a
/// public URL. That is what lets a local file be sent without an upload step.
pub fn data_uri(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64_encode(bytes))
}

/// A one-file `multipart/form-data` body, returned with its content type.
///
/// Hand-built because `renzora::net` carries a body as bytes and has no form
/// encoder. One part with a fixed boundary is all Tripo's upload takes; the
/// boundary cannot collide with the file because it is checked against it.
pub fn multipart_file(field: &str, filename: &str, mime: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    let mut boundary = String::from("renzora-ai-mesh-boundary");
    while bytes
        .windows(boundary.len())
        .any(|w| w == boundary.as_bytes())
    {
        boundary.push('x');
    }
    let mut body = Vec::with_capacity(bytes.len() + 256);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {mime}\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_every_tail_length() {
        for len in 0..10 {
            let bytes: Vec<u8> = (0..len as u8).map(|b| b.wrapping_mul(37)).collect();
            let encoded = base64_encode(&bytes);
            assert_eq!(encoded.len() % 4, 0);
            assert_eq!(base64_decode(&encoded).unwrap(), bytes);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
    }

    #[test]
    fn errors_read_each_providers_shape() {
        assert_eq!(
            error_message(400, r#"{"code":2010,"message":"No credit","suggestion":"Top up"}"#),
            "No credit (Top up)"
        );
        assert_eq!(
            error_message(422, r#"{"detail":[{"msg":"field required"}]}"#),
            "field required"
        );
        assert!(error_message(401, "not json").starts_with("HTTP 401."));
    }
}
