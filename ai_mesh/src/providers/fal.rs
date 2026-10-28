//! fal.ai's queue (`queue.fal.run`), for the models it hosts.
//!
//! Submit with `POST queue.fal.run/<model id>`, which answers with a
//! `status_url` and a `response_url`; poll the first until `COMPLETED`, then
//! read the second. The URLs come back from the submit call and are used as
//! given: fal builds them from the owner and app only, not the full model id,
//! and a hand-built one that includes the endpoint path is a `405`.

use std::time::Duration;

use renzora::serde_json::{json, Value};

use super::{poll, Input, Output, Report, Spec, LOW_POLY_FACES};
use crate::http::{self, Auth};

const QUEUE: &str = "https://queue.fal.run";
const POLL_EVERY: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
pub enum Model {
    Hunyuan,
    Trellis,
}

pub fn run(model: Model, spec: &Spec, report: &Report) -> Result<Output, String> {
    let auth = Auth::FalKey(&spec.key);
    let (id, body) = request(model, spec)?;

    let submitted = http::post_json(&format!("{QUEUE}/{id}"), auth, &body)?;
    let status_url = field(&submitted, "status_url")?;
    let response_url = field(&submitted, "response_url")?;
    // Optional on fal's side, so its absence costs only the remote cancel.
    let cancel_url = field(&submitted, "cancel_url").ok();
    let cancel = || {
        if let Some(url) = &cancel_url {
            http::fire("PUT", url, auth);
        }
    };

    poll(POLL_EVERY, report, cancel, || {
        let status = http::get_json(&status_url, auth)?;
        match status.get("status").and_then(Value::as_str).unwrap_or("") {
            "COMPLETED" => Ok(Some(())),
            "IN_QUEUE" => {
                let label = match status.get("queue_position").and_then(Value::as_u64) {
                    Some(n) if n > 0 => format!("queued, {n} ahead"),
                    _ => "queued".to_string(),
                };
                (report.progress)(None, &label);
                Ok(None)
            }
            _ => {
                // fal reports no percentage, only that the runner has it.
                (report.progress)(None, "generating");
                Ok(None)
            }
        }
    })?;

    let result = http::get_json(&response_url, auth)?;
    // A failed request still reaches COMPLETED; the reason is on the result.
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    if let Some(thumb) = result.pointer("/thumbnail/url").and_then(Value::as_str) {
        (report.thumbnail)(thumb.to_string());
    }
    let glb_url = result
        .pointer("/model_glb/url")
        .and_then(Value::as_str)
        .ok_or("fal finished but the result has no model_glb")?;

    (report.progress)(Some(100), "downloading");
    Ok(Output {
        glb: http::get_bytes(glb_url)?,
    })
}

/// The endpoint id and request body for one job.
fn request(model: Model, spec: &Spec) -> Result<(&'static str, Value), String> {
    match (model, &spec.input) {
        (Model::Hunyuan, Input::Text(prompt)) => {
            let mut body = json!({ "prompt": prompt, "enable_pbr": spec.pbr });
            hunyuan_budget(&mut body, spec.low_poly);
            Ok(("fal-ai/hunyuan3d-v3/text-to-3d", body))
        }
        (Model::Hunyuan, Input::Image { bytes, mime, .. }) => {
            let mut body = json!({
                "input_image_url": http::data_uri(mime, bytes),
                "enable_pbr": spec.pbr,
            });
            hunyuan_budget(&mut body, spec.low_poly);
            Ok(("fal-ai/hunyuan3d-v3/image-to-3d", body))
        }
        (Model::Trellis, Input::Image { bytes, mime, .. }) => {
            let mut body = json!({ "image_url": http::data_uri(mime, bytes) });
            if spec.low_poly {
                body["decimation_target"] = json!(LOW_POLY_FACES);
            }
            // TRELLIS always bakes PBR maps; there is no switch for it to take.
            Ok(("fal-ai/trellis-2", body))
        }
        (Model::Trellis, Input::Text(_)) => {
            Err("TRELLIS 2 generates from an image only; switch to Image".to_string())
        }
    }
}

/// Hunyuan's `LowPoly` mode, which retopologises rather than decimates. Its
/// `face_count` floor is 40,000, so that is the closest it comes to the shared
/// budget.
fn hunyuan_budget(body: &mut Value, low_poly: bool) {
    if low_poly {
        body["generate_type"] = json!("LowPoly");
        body["face_count"] = json!(LOW_POLY_FACES.max(40_000));
    }
}

fn field(reply: &Value, name: &str) -> Result<String, String> {
    reply
        .get(name)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| format!("fal's queue did not return a {name}"))
}
