//! A Hunyuan3D 2.1 server on the user's own machine (`api_server.py`, port
//! 8081 by default). Free, offline, and as fast as the GPU it runs on.
//!
//! `POST /send` with a base64 image starts a job and returns its `uid`;
//! `GET /status/<uid>` answers `processing`, then `texturing`, then `completed`
//! with the finished GLB inline as `model_base64`.

use std::time::Duration;

use renzora::serde_json::{json, Value};

use super::{poll, Input, Output, Report, Spec, LOW_POLY_FACES};
use crate::http::{self, Auth};

const POLL_EVERY: Duration = Duration::from_secs(2);

pub fn run(spec: &Spec, report: &Report) -> Result<Output, String> {
    let Input::Image { bytes, .. } = &spec.input else {
        return Err("the local Hunyuan3D server generates from an image only".to_string());
    };
    let base = spec.local_url.trim_end_matches('/');
    if base.is_empty() {
        return Err("set the local server URL in Settings > Plugins > AI Mesh".to_string());
    }

    let mut body = json!({
        // A bare base64 string, not a data URI: the server decodes the field
        // directly and a `data:` prefix makes it fail.
        "image": http::base64_encode(bytes),
        // The server's default is an untextured mesh, which is never what a
        // user generating an asset for a scene wants.
        "texture": true,
    });
    if spec.low_poly {
        body["face_count"] = json!(LOW_POLY_FACES);
    }

    let reply = http::post_json(&format!("{base}/send"), Auth::None, &body)
        .map_err(|e| format!("{e} (is api_server.py running at {base}?)"))?;
    let uid = reply
        .get("uid")
        .and_then(Value::as_str)
        .ok_or("the server did not return a job id")?
        .to_string();

    let url = format!("{base}/status/{uid}");
    // `api_server.py` has no cancel route; the GPU finishes the job and the
    // result is never collected.
    let encoded = poll(POLL_EVERY, report, || {}, || {
        let status = http::get_json(&url, Auth::None)?;
        match status.get("status").and_then(Value::as_str).unwrap_or("") {
            "completed" => status
                .get("model_base64")
                .and_then(Value::as_str)
                .map(|m| Some(m.to_string()))
                .ok_or_else(|| "the server finished but sent no model".to_string()),
            "error" => Err(status
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the server reported an error")
                .to_string()),
            "texturing" => {
                (report.progress)(Some(60), "texturing");
                Ok(None)
            }
            _ => {
                (report.progress)(Some(20), "shaping");
                Ok(None)
            }
        }
    })?;

    Ok(Output {
        glb: http::base64_decode(&encoded)?,
    })
}
