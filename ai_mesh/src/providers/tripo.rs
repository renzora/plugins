//! Tripo (`openapi.tripo3d.com/v3`).
//!
//! One task per model, created at `/generation/text-to-model` or
//! `/generation/image-to-model` and polled at `/tasks/<id>`. Every reply is
//! wrapped as `{"code": 0, "data": {...}}`.
//!
//! An image goes up first through `POST /files` and is referenced by the
//! `file_token` that returns. Tripo's `input` also takes a public URL, but not a
//! data URI, and a file on the user's disk has no public URL.

use std::time::Duration;

use renzora::serde_json::{json, Value};

use super::{percent, poll, Input, Output, Report, Spec, LOW_POLY_FACES};
use crate::http::{self, Auth};

const API: &str = "https://openapi.tripo3d.com/v3";

/// Tripo asks for no more than one status request a second and recommends two.
const POLL_EVERY: Duration = Duration::from_secs(2);

/// The P-series model: Tripo's game-asset line, with clean low-poly topology and
/// a face budget it actually honours. The H-series default treats `face_limit`
/// as a ceiling on a million-triangle generation and decimates down to it.
const LOW_POLY_MODEL: &str = "P1-20260311";

pub fn run(spec: &Spec, report: &Report) -> Result<Output, String> {
    let auth = Auth::Bearer(&spec.key);
    let (endpoint, mut body) = match &spec.input {
        Input::Text(prompt) => (
            format!("{API}/generation/text-to-model"),
            json!({ "prompt": prompt }),
        ),
        Input::Image {
            bytes,
            mime,
            filename,
        } => {
            (report.progress)(Some(0), "uploading image");
            let token = upload(auth, filename, mime, bytes)?;
            // Uploads are free, generation is not: last chance to stop.
            report.check()?;
            (
                format!("{API}/generation/image-to-model"),
                json!({ "input": token }),
            )
        }
    };
    body["texture"] = json!(true);
    body["pbr"] = json!(spec.pbr);
    if spec.low_poly {
        body["model"] = json!(LOW_POLY_MODEL);
        body["face_limit"] = json!(LOW_POLY_FACES);
    }

    let reply = http::post_json(&endpoint, auth, &body)?;
    let id = data(&reply)?
        .get("task_id")
        .and_then(Value::as_str)
        .ok_or("Tripo did not return a task id")?
        .to_string();

    let url = format!("{API}/tasks/{id}");
    // Tripo documents no cancel endpoint, so a cancel stops here and the task
    // runs on at Tripo's end. It charges on success, so a cancelled job that
    // would have succeeded may still cost its credits.
    let glb_url = poll(POLL_EVERY, report, || {}, || {
        let reply = http::get_json(&url, auth)?;
        let task = data(&reply)?;
        match task.get("status").and_then(Value::as_str).unwrap_or("") {
            "success" => {
                if let Some(thumb) = task.pointer("/output/rendered_image_url").and_then(Value::as_str) {
                    (report.thumbnail)(thumb.to_string());
                }
                task.pointer("/output/model_url")
                    .and_then(Value::as_str)
                    .map(|u| Some(u.to_string()))
                    .ok_or_else(|| "Tripo finished but published no model".to_string())
            }
            "banned" => Err("Tripo refused the input under its content policy".to_string()),
            "expired" => Err("the task expired before it could be downloaded".to_string()),
            "cancelled" => Err("the task was cancelled on Tripo's side".to_string()),
            "failed" => Err(task
                .get("error_message")
                .and_then(Value::as_str)
                .unwrap_or("Tripo reported the task as failed")
                .to_string()),
            status => {
                let label = if status == "queued" { "queued" } else { "generating" };
                (report.progress)(percent(task.get("progress")), label);
                Ok(None)
            }
        }
    })?;

    // Straight away: Tripo's model URLs expire five minutes after the task
    // finishes, so this is not a step that can wait for anything.
    (report.progress)(Some(100), "downloading");
    Ok(Output {
        glb: http::get_bytes(&glb_url)?,
    })
}

fn upload(auth: Auth, filename: &str, mime: &str, bytes: &[u8]) -> Result<String, String> {
    let (content_type, body) = http::multipart_file("file", filename, mime, bytes);
    let reply = http::post_raw(&format!("{API}/files"), auth, &content_type, body)?;
    data(&reply)?
        .get("file_token")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| "Tripo accepted the image but returned no file token".to_string())
}

/// Unwrap `{"code": 0, "data": ...}`.
///
/// A non-zero `code` with a 200 is how Tripo reports some refusals, so it is
/// checked here rather than trusting the HTTP status alone.
fn data(reply: &Value) -> Result<&Value, String> {
    match reply.get("code").and_then(Value::as_i64) {
        Some(0) | None => reply
            .get("data")
            .ok_or_else(|| "Tripo replied without a data field".to_string()),
        Some(code) => Err(reply
            .get("message")
            .and_then(Value::as_str)
            .map(|m| format!("{m} (Tripo error {code})"))
            .unwrap_or_else(|| format!("Tripo error {code}"))),
    }
}
