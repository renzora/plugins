//! Profiling tools: record the editor or a running game, and ask where the
//! time goes.
//!
//! The profile lives in the companion process, which records it at no cost to
//! the editor's own frame budget beyond the recording itself. These tools ask
//! it through the engine's companion job channel (`renzora::CompanionJobs`):
//! the question goes out as a job, the companion answers from its live capture,
//! and the answer comes back as a `CompanionJobDone` a frame or two later. So,
//! like screenshots, every call here hands its reply channel over and answers
//! from [`collect`].

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use renzora::job_args::{ProfilerQuery, ProfilerRecord};
use renzora::serde_json::{self, Value};
use renzora::{CompanionJobDone, CompanionJobId, CompanionJobs};

use crate::ToolReply;

/// Shorter than the RPC's own timeout, so the caller hears why.
const DEADLINE: Duration = Duration::from_secs(15);

pub struct PendingQuestion {
    job: CompanionJobId,
    reply: Sender<ToolReply>,
    deadline: Instant,
}

#[derive(Resource, Default)]
pub struct PendingQuestions(pub Vec<PendingQuestion>);

pub fn build(app: &mut App) {
    // The host registers these too; registering them here as well means the
    // tools fail in words rather than the system failing to run when the
    // companion was switched off.
    renzora::init_companion_jobs(app);
    app.init_resource::<PendingQuestions>().add_systems(Update, collect);
}

pub fn is_profiler_tool(name: &str) -> bool {
    matches!(name, "profiler_record" | "profiler_status" | "profiler_summary" | "profiler_spikes")
}

/// Send the question; [`collect`] answers it.
pub fn start(world: &mut World, name: &str, args: &Value, reply: Sender<ToolReply>) {
    let seconds = args.get("seconds").and_then(Value::as_f64).map(|s| s as f32);
    let limit = args.get("limit").and_then(Value::as_u64).map(|l| l as usize);
    let over_ms = args.get("over_ms").and_then(Value::as_f64).map(|s| s as f32);
    let query = ProfilerQuery { seconds, limit, over_ms };

    let Some(mut jobs) = world.get_resource_mut::<CompanionJobs>() else {
        let _ = reply.send(ToolReply::Failed("this editor has no companion".into()));
        return;
    };
    let submitted = match name {
        "profiler_record" => jobs.submit(
            "profiler.record",
            "profiler",
            &ProfilerRecord {
                on: args.get("on").and_then(Value::as_bool).unwrap_or(true),
                game: args.get("game").and_then(Value::as_str).map(str::to_string),
            },
        ),
        "profiler_status" => jobs.submit("profiler.status", "profiler", &query),
        "profiler_summary" => jobs.submit("profiler.summary", "profiler", &query),
        "profiler_spikes" => jobs.submit("profiler.spikes", "profiler", &query),
        other => {
            let _ = reply.send(ToolReply::Failed(format!("no such tool: {other}")));
            return;
        }
    };
    let Some(job) = submitted else {
        let _ = reply.send(ToolReply::Failed(
            "the companion is not running (it starts with the editor a few seconds after launch; RENZORA_NO_COMPANION switches it off)"
                .into(),
        ));
        return;
    };
    world.resource_mut::<PendingQuestions>().0.push(PendingQuestion {
        job,
        reply,
        deadline: Instant::now() + DEADLINE,
    });
}

fn collect(mut done: MessageReader<CompanionJobDone>, mut pending: ResMut<PendingQuestions>) {
    for answer in done.read() {
        let Some(at) = pending.0.iter().position(|q| q.job == answer.id) else {
            continue;
        };
        let question = pending.0.swap_remove(at);
        let _ = question.reply.send(match &answer.result {
            Ok(text) => ToolReply::Text(readable(text)),
            Err(why) => ToolReply::Failed(why.clone()),
        });
    }
    let now = Instant::now();
    pending.0.retain(|q| {
        if now < q.deadline {
            return true;
        }
        let _ = q.reply.send(ToolReply::Failed("the companion did not answer".into()));
        false
    });
}

/// JSON answers indented, so they read as a report; anything else as it is.
fn readable(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| text.to_string())
}
