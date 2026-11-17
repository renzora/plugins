//! `project_load`: how long a project takes to load, stage by stage.
//!
//! The engine records every load in [`renzora::load_timeline::LoadTimeline`]:
//! scene parsed, models loaded, viewport camera ready, textures, virtual
//! geometry, environment lighting, with the time each was reached and every
//! frame that stalled the editor. This tool reads it back, and with `reload`
//! it reopens the project first and waits for the new load to finish, which
//! is what makes "loading feels slower" a number that can be compared before
//! and after a change.
//!
//! The reopen goes through [`renzora::RequestSwitchProject`], the same path as
//! File > Recent Projects, so an unsaved scene gets the editor's own "save
//! before switching?" prompt rather than being thrown away by a tool call.

use bevy::prelude::*;
use renzora::load_timeline::LoadTimeline;
use renzora::serde_json::{json, Value};
use renzora::{CurrentProject, SplashState};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::ToolReply;

pub const TOOL: &str = "project_load";
pub const MEMORY_TOOL: &str = "memory_report";

/// What the editor holds in memory now: the process total, and what each kind
/// of asset accounts for in it. Answered at once: the breakdown is kept
/// current by the engine twice a second.
pub fn memory_report(world: &World) -> ToolReply {
    match world.get_resource::<renzora::load_timeline::AssetMemory>() {
        Some(memory) => ToolReply::Text(memory.report()),
        None => ToolReply::Failed(
            "this editor does not report asset memory; it predates the memory breakdown".into(),
        ),
    }
}

/// How long a reload is waited on. Under the call's own timeout in `rpc.rs`,
/// so the caller hears this tool's explanation rather than a generic one.
const DEADLINE: Duration = Duration::from_secs(170);

/// A reload in flight: who to answer, and which load it is waiting for.
struct PendingLoad {
    reply: Sender<ToolReply>,
    /// The timeline generation when the reload was asked for; the load being
    /// waited on is any later one.
    before: u32,
    asked: Instant,
}

#[derive(Resource, Default)]
pub struct PendingLoads(Vec<PendingLoad>);

pub fn build(app: &mut App) {
    app.init_resource::<PendingLoads>().add_systems(Update, collect_loads);
}

pub fn catalogue() -> Value {
    json!([{
        "name": MEMORY_TOOL,
        "description": "How much memory the editor holds right now and what in: committed and resident totals, then each kind of asset (mesh and image CPU copies, virtual geometry files) with its count and size, largest first, how much of the total is outside the assets (GPU driver copies and staging, render buffers, pipelines, the allocator), and an estimate of the images' size on the GPU. Measure before and after an action to see what it allocates and whether it gives it back.",
        "inputSchema": { "type": "object", "properties": {} }
    }, {
        "name": TOOL,
        "description": "How long the project took to load, stage by stage: scene parsed, models loaded and spawned, viewport camera ready, editor opened, textures loaded, virtual geometry ready, environment lighting filled, each with its time and the gap since the stage before, plus the longest frame and any frames that stalled the editor. With no arguments, the last load since the editor started. With reload, reopens the open project (or the one named by project) the way File > Recent Projects does, unsaved-changes prompt included, waits for it to finish and reports that load; use it to measure a load before and after a change.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "reload": { "type": "boolean", "description": "Reopen the project and time the new load. Takes as long as the load, up to about three minutes." },
                "wait": { "type": "boolean", "description": "Without reload: wait for the load in progress, or the next one to start, and report it. For a project opened from the launcher, which is the cold load a person actually waits through: call this, then click the project's tile with click." },
                "project": { "type": "string", "description": "A project folder (or its project.toml) to open and time instead of the open one. Implies reload." }
            }
        }
    }])
}

pub fn start(world: &mut World, args: &Value, reply: Sender<ToolReply>) {
    let project = args.get("project").and_then(Value::as_str).filter(|p| !p.trim().is_empty());
    let reload = project.is_some() || args.get("reload").and_then(Value::as_bool).unwrap_or(false);
    let wait = args.get("wait").and_then(Value::as_bool).unwrap_or(false);
    let Some(timeline) = world.get_resource::<LoadTimeline>() else {
        let _ = reply.send(ToolReply::Failed(
            "this editor does not record project loads; it predates the load timeline".into(),
        ));
        return;
    };
    if !reload && wait {
        // The load in progress, or else the next one to start: opening a
        // project from the launcher is a click, not something this tool does.
        let generation = timeline.generation();
        let before = if timeline.is_running() { generation.wrapping_sub(1) } else { generation };
        world.resource_mut::<PendingLoads>().0.push(PendingLoad {
            reply,
            before,
            asked: Instant::now(),
        });
        return;
    }
    if !reload {
        let _ = reply.send(ToolReply::Text(format!("{}\n{}", phase(world), timeline.report())));
        return;
    }
    let before = timeline.generation();

    let root = match project {
        Some(path) => {
            let path = PathBuf::from(path.trim());
            if path.file_name().is_some_and(|n| n == "project.toml") {
                path.parent().map(PathBuf::from).unwrap_or(path)
            } else {
                path
            }
        }
        None => match world.get_resource::<CurrentProject>() {
            Some(current) => current.path.clone(),
            None => {
                let _ = reply.send(ToolReply::Failed(
                    "no project is open to reload; name one with project".into(),
                ));
                return;
            }
        },
    };
    if !root.join("project.toml").is_file() {
        let _ = reply.send(ToolReply::Failed(format!(
            "{} is not a project: it has no project.toml",
            root.display()
        )));
        return;
    }

    world.insert_resource(renzora::RequestSwitchProject(root));
    world.resource_mut::<PendingLoads>().0.push(PendingLoad {
        reply,
        before,
        asked: Instant::now(),
    });
}

/// Answer each reload once its load has finished, or once it has run out of
/// time, with whatever had been recorded by then.
fn collect_loads(world: &mut World) {
    if world.resource::<PendingLoads>().0.is_empty() {
        return;
    }
    let (generation, finished, report) = match world.get_resource::<LoadTimeline>() {
        Some(t) => (t.generation(), t.finished().is_some(), t.report()),
        None => return,
    };
    let phase = phase(world);
    let mut pending = std::mem::take(&mut world.resource_mut::<PendingLoads>().0);
    pending.retain(|load| {
        let started = generation != load.before;
        if started && finished {
            let _ = load.reply.send(ToolReply::Text(report.clone()));
            return false;
        }
        if load.asked.elapsed() < DEADLINE {
            return true;
        }
        let text = if started {
            format!("The load had not finished after {}s.\n{report}", DEADLINE.as_secs())
        } else {
            format!(
                "No load started in {}s. After a reload, the editor may be showing its \"save before switching?\" prompt for unsaved changes; answer it and call project_load with wait.\n{phase}",
                DEADLINE.as_secs()
            )
        };
        let _ = load.reply.send(ToolReply::Text(text));
        false
    });
    world.resource_mut::<PendingLoads>().0.extend(pending);
}

/// Which screen the editor is on, in the words a person would use.
pub fn phase(world: &World) -> String {
    let phase = match world.get_resource::<State<SplashState>>().map(|s| *s.get()) {
        Some(SplashState::Splash) => "the launcher (no project open)",
        Some(SplashState::Loading) => "the loading screen",
        Some(SplashState::Editor) => "the editor",
        None => "unknown",
    };
    format!("phase: {phase}")
}
