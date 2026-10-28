//! Meshy (`api.meshy.ai`).
//!
//! Text is two tasks: a **preview** that builds untextured geometry, then a
//! **refine** that textures it. Image is one task that does both. All three
//! answer the create call with `{"result": "<task id>"}` and are polled at
//! `GET <endpoint>/<id>` until `status` is `SUCCEEDED`, with `model_urls.glb`
//! holding a signed download link.

use std::time::Duration;

use renzora::serde_json::{json, Value};

use super::{percent, poll, Input, Output, Report, Spec, LOW_POLY_FACES};
use crate::http::{self, Auth};

const API: &str = "https://api.meshy.ai";
const POLL_EVERY: Duration = Duration::from_secs(3);

pub fn run(spec: &Spec, report: &Report) -> Result<Output, String> {
    let auth = Auth::Bearer(&spec.key);
    let glb_url = match &spec.input {
        Input::Text(prompt) => {
            let endpoint = format!("{API}/openapi/v2/text-to-3d");
            let mut preview = json!({
                "mode": "preview",
                "prompt": prompt,
                "target_formats": ["glb"],
            });
            low_poly(&mut preview, spec.low_poly);
            let id = create(&endpoint, auth, &preview)?;
            // The preview is half the job and has no texture, so its progress is
            // scaled into the first half of the bar rather than reaching 100% on
            // a model that is still grey.
            wait(&endpoint, &id, auth, report, "shaping", 0, 50)?;
            // The refine is a second paid task; a cancel that arrived while the
            // preview was finishing must not start it.
            report.check()?;

            let refine = json!({
                "mode": "refine",
                "preview_task_id": id,
                "enable_pbr": spec.pbr,
                "target_formats": ["glb"],
            });
            let id = create(&endpoint, auth, &refine)?;
            wait(&endpoint, &id, auth, report, "texturing", 50, 100)?
        }
        Input::Image { bytes, mime, .. } => {
            let endpoint = format!("{API}/openapi/v1/image-to-3d");
            let mut body = json!({
                "image_url": http::data_uri(mime, bytes),
                "should_texture": true,
                "enable_pbr": spec.pbr,
                "target_formats": ["glb"],
            });
            low_poly(&mut body, spec.low_poly);
            let id = create(&endpoint, auth, &body)?;
            wait(&endpoint, &id, auth, report, "generating", 0, 100)?
        }
    };

    (report.progress)(Some(100), "downloading");
    Ok(Output {
        glb: http::get_bytes(&glb_url)?,
    })
}

/// Remesh to a fixed budget. Meshy's own default keeps the raw generation,
/// which for `meshy-6` and later is hundreds of thousands of triangles.
fn low_poly(body: &mut Value, on: bool) {
    if on {
        body["should_remesh"] = json!(true);
        body["target_polycount"] = json!(LOW_POLY_FACES);
    }
}

fn create(endpoint: &str, auth: Auth, body: &Value) -> Result<String, String> {
    let reply = http::post_json(endpoint, auth, body)?;
    reply
        .get("result")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| "Meshy did not return a task id".to_string())
}

/// Poll one task to completion and return its GLB URL. Its own 0-100 is mapped
/// onto `from..to` of the job's bar.
fn wait(
    endpoint: &str,
    id: &str,
    auth: Auth,
    report: &Report,
    stage: &str,
    from: u8,
    to: u8,
) -> Result<String, String> {
    let url = format!("{endpoint}/{id}");
    let mut thumb_sent = false;
    // Deleting a task that is still PENDING refunds its credits. One already
    // IN_PROGRESS answers 409 and runs to completion; it is simply never
    // downloaded.
    let delete = || http::fire("DELETE", &url, auth);
    poll(POLL_EVERY, report, delete, || {
        let task = http::get_json(&url, auth)?;
        if !thumb_sent {
            if let Some(thumb) = task.get("thumbnail_url").and_then(Value::as_str) {
                if !thumb.is_empty() {
                    (report.thumbnail)(thumb.to_string());
                    thumb_sent = true;
                }
            }
        }
        match task.get("status").and_then(Value::as_str).unwrap_or("") {
            "SUCCEEDED" => task
                .pointer("/model_urls/glb")
                .and_then(Value::as_str)
                .map(|u| Some(u.to_string()))
                .ok_or_else(|| "Meshy finished but published no GLB".to_string()),
            "FAILED" => Err(task
                .pointer("/task_error/message")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                .unwrap_or("Meshy reported the task as failed")
                .to_string()),
            "CANCELED" => Err("the task was cancelled on Meshy's side".to_string()),
            status => {
                let own = percent(task.get("progress")).unwrap_or(0) as u32;
                let scaled = from as u32 + own * (to - from) as u32 / 100;
                let label = if status == "PENDING" { "queued" } else { stage };
                (report.progress)(Some(scaled as u8), label);
                Ok(None)
            }
        }
    })
}
