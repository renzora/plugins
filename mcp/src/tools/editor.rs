//! Tools that drive the editor itself: its camera, its menus, its selection,
//! play mode, saving and undo.

use bevy::prelude::*;
use renzora::serde_json::Value;
use renzora::undo::{redo_once, undo_once, UndoContext, UndoStacks};
use renzora::{
    AutoSaveSettings, CameraViewRequest, CreateNodeRequested, EditorSelection, ExportRequested, ImportPick,
    ImportRequested, IsolationMode, NewSceneRequested, OpenCodeEditorFile, OpenScenePathRequested,
    OpenSceneRequested, OpenUiTemplateFile, PlayModeState, SaveAsSceneRequested,
    SaveSceneRequested, ToggleCommandPaletteRequested, ToggleSettingsRequested, TutorialRequested,
    UpdateRequested,
};

use super::{arg_entity, arg_vec3, entity_id, value_to_entity};
use crate::ToolReply;

/// Move the editor's viewport camera.
///
/// Through [`CameraViewRequest`] rather than by writing the camera's
/// `Transform`, which looks like it works and does not: the controller recomputes
/// that transform from its orbit state every frame, so a write to it is gone
/// before the next render. The request is the supported seam and is what the
/// View menu and the F key both go through.
pub(super) fn set_view(world: &mut World, args: &Value) -> ToolReply {
    if let Some(entity) = arg_entity(args, "frame") {
        if world.get_entity(entity).is_err() {
            return ToolReply::Failed(format!("{} is not a live entity", entity_id(entity)));
        }
        world.insert_resource(CameraViewRequest::Frame(entity));
        return ToolReply::Text(format!("framing {}", entity_id(entity)));
    }

    if args
        .get("frame_all")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        world.insert_resource(CameraViewRequest::FrameAll);
        return ToolReply::Text("framing the whole scene".into());
    }

    let (Some(eye), Some(target)) = (arg_vec3(args, "eye"), arg_vec3(args, "target")) else {
        return ToolReply::Failed(
            "set_view needs either an eye and a target, a frame entity, or frame_all".into(),
        );
    };
    world.insert_resource(CameraViewRequest::LookAt { eye, target });
    ToolReply::Text(format!(
        "looking from [{}, {}, {}] at [{}, {}, {}]",
        eye.x, eye.y, eye.z, target.x, target.y, target.z
    ))
}

/// Ask the editor for one of the things its menus ask for.
///
/// Every one of these is a marker resource the editor drains next frame, which
/// is the seam the File, Edit and View menus themselves go through: a menu item
/// does not call the exporter, it inserts `ExportRequested`. Doing the same
/// thing here means this tool is not a parallel path that can rot, it is the
/// same path with a different caller.
///
/// Typed rather than reflected, which was the surprise. Bevy 0.19's
/// `ReflectResource` is a marker with no functionality and `insert_resource` is
/// generic over the type, so there is no by-name route for resources the way
/// there is for components. What makes this reachable at all is that the request
/// types live in the contract crate, which a plugin links. The rule that follows
/// is worth stating: a request a plugin should be able to make has to be
/// declared in `renzora`, not in the crate that consumes it.
pub(super) fn editor_request(world: &mut World, args: &Value) -> ToolReply {
    let Some(request) = args.get("request").and_then(Value::as_str) else {
        return ToolReply::Failed(format!("editor_request needs a request; known: {REQUESTS}"));
    };
    let path = || {
        args.get("path")
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from)
    };

    match request {
        "save_scene" => world.insert_resource(SaveSceneRequested),
        "save_scene_as" => world.insert_resource(SaveAsSceneRequested),
        "new_scene" => world.insert_resource(NewSceneRequested),
        "open_scene" => world.insert_resource(OpenSceneRequested),
        "open_scene_path" => match path() {
            Some(path) => world.insert_resource(OpenScenePathRequested(path)),
            None => return ToolReply::Failed("open_scene_path needs a path".into()),
        },
        "open_code_file" => match path() {
            Some(path) => world.insert_resource(OpenCodeEditorFile { path }),
            None => return ToolReply::Failed("open_code_file needs a path".into()),
        },
        "open_ui_template" => match path() {
            Some(path) => world.insert_resource(OpenUiTemplateFile { path }),
            None => return ToolReply::Failed("open_ui_template needs a path".into()),
        },
        "export" => world.insert_resource(ExportRequested),
        "import_files" => world.insert_resource(ImportRequested(ImportPick::Files)),
        "import_folder" => world.insert_resource(ImportRequested(ImportPick::Folder)),
        "check_updates" => world.insert_resource(UpdateRequested),
        "tutorial" => world.insert_resource(TutorialRequested),
        "settings" => world.insert_resource(ToggleSettingsRequested),
        "create_node" => world.insert_resource(CreateNodeRequested),
        "command_palette" => world.insert_resource(ToggleCommandPaletteRequested),
        "isolation" => {
            let active = args
                .get("active")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| {
                    !world
                        .get_resource::<IsolationMode>()
                        .is_some_and(|mode| mode.active)
                });
            world.insert_resource(IsolationMode { active });
        }
        other => {
            return ToolReply::Failed(format!("no such request: {other}; known: {REQUESTS}"));
        }
    }

    // Four of these open a native file dialog, which is modal and blocks the
    // editor until a person answers it. Saying so in the reply matters more than
    // it looks: an agent that fires `open_scene` and then waits for the editor to
    // respond has hung the editor and itself, and the honest reading of silence
    // afterwards is "go and look at the screen".
    let modal = matches!(
        request,
        "open_scene" | "save_scene_as" | "import_files" | "import_folder"
    );
    ToolReply::Text(if modal {
        format!("asked for {request}; it opens a file dialog, so the editor is now waiting on a person")
    } else {
        format!("asked for {request}")
    })
}

const REQUESTS: &str = "save_scene, save_scene_as, new_scene, open_scene, open_scene_path, \
open_code_file, open_ui_template, export, import_files, import_folder, check_updates, tutorial, \
settings, create_node, command_palette, isolation";

pub(super) fn select_entities(world: &mut World, args: &Value) -> ToolReply {
    let Some(list) = args.get("entities").and_then(Value::as_array) else {
        return ToolReply::Failed("select_entities needs an entities array".into());
    };
    let mut entities = Vec::with_capacity(list.len());
    for raw in list {
        let Some(entity) = value_to_entity(raw) else {
            return ToolReply::Failed(format!("not an entity id: {raw}"));
        };
        entities.push(entity);
    }

    let Some(selection) = world.get_resource::<EditorSelection>() else {
        return ToolReply::Failed("there is no editor selection to set".into());
    };
    let count = entities.len();
    selection.set_multiple(entities);
    ToolReply::Text(match count {
        0 => "cleared the selection".to_string(),
        1 => "selected 1 entity".to_string(),
        n => format!("selected {n} entities"),
    })
}

pub(super) fn set_play_mode(world: &mut World, args: &Value) -> ToolReply {
    let Some(mode) = args.get("mode").and_then(Value::as_str) else {
        return ToolReply::Failed("set_play_mode needs a mode".into());
    };
    let Some(mut play) = world.get_resource_mut::<PlayModeState>() else {
        return ToolReply::Failed("play mode is not available in this build".into());
    };

    // Requests rather than direct state writes: the editor owns the transition
    // (cameras, chrome, script startup) and reads these flags on its next frame.
    match mode {
        "play" => play.request_play = true,
        "pause" => play.request_pause = true,
        "stop" => play.request_stop = true,
        "simulate" => play.request_simulate = true,
        other => return ToolReply::Failed(format!("unknown mode: {other}")),
    }
    ToolReply::Text(format!("asked the editor to {mode}"))
}

/// Save the open scene tab, as Ctrl+S does.
///
/// The reply spells out where it goes because this plugin cannot see the tab:
/// the tab state lives in a crate a plugin cannot link. A tab with a file saves
/// to that file; a tab that has never been saved opens a Save As dialog, which
/// blocks the whole editor until a person answers it.
pub(super) fn save_scene(world: &mut World) -> ToolReply {
    world.insert_resource(SaveSceneRequested);
    ToolReply::Text(
        "asked the editor to save the open scene tab to its own file. If this tab has never been saved, a Save As dialog is now open and the editor is waiting on a person; read_console shows the path once it is written".into(),
    )
}

/// Pause or resume autosave for this session.
///
/// For an agent's own test runs. Autosave writes whatever is in the open tab
/// into that tab's file every few minutes, so experimental edits land in a real
/// scene; and in a tab that has never been saved it opens a Save As dialog,
/// which blocks the whole editor until a person answers it.
///
/// Only the live resource changes. The preference file is left alone, so the
/// next launch comes back with the user's own setting whatever an agent did.
pub(super) fn set_autosave(world: &mut World, args: &Value) -> ToolReply {
    let Some(mut settings) = world.get_resource_mut::<AutoSaveSettings>() else {
        return ToolReply::Failed("this editor has no autosave".into());
    };
    if let Some(enabled) = args.get("enabled").and_then(Value::as_bool) {
        settings.enabled = enabled;
    }
    ToolReply::Text(format!(
        "autosave is {} for this session (every {}s); the saved preference is unchanged",
        if settings.enabled { "on" } else { "off" },
        settings.interval_secs
    ))
}

/// Step the scene's undo stack back or forward, as Ctrl+Z and Ctrl+Y do.
///
/// Always the Scene stack, whichever the editor has active: that is where every
/// edit these tools make is recorded, and an agent undoing its own spawn while
/// the user happens to have the material editor focused must not reach into
/// that editor's history instead. The active stack is put back afterwards.
///
/// The labels come back in the reply because this is the user's history too,
/// not only the agent's: an undo that went one step further than meant is
/// visible in the answer rather than discovered later.
pub(super) fn undo_redo(world: &mut World, args: &Value) -> ToolReply {
    let redo = args.get("redo").and_then(Value::as_bool).unwrap_or(false);
    let steps = args
        .get("steps")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 50);
    let Some(previous) = world.get_resource::<UndoStacks>().map(|s| s.active.clone()) else {
        return ToolReply::Failed("this editor has no undo history".into());
    };
    world.resource_mut::<UndoStacks>().active = UndoContext::Scene;

    let mut done = Vec::new();
    for _ in 0..steps {
        let (undo_labels, redo_labels) = world.resource::<UndoStacks>().labels(&UndoContext::Scene);
        let next = if redo { redo_labels.last() } else { undo_labels.last() }.cloned();
        let stepped = if redo { redo_once(world) } else { undo_once(world) };
        if !stepped {
            break;
        }
        done.push(next.unwrap_or_else(|| "edit".into()));
    }
    world.resource_mut::<UndoStacks>().active = previous;

    let verb = if redo { "redid" } else { "undid" };
    if done.is_empty() {
        return ToolReply::Failed(format!("nothing to {}", if redo { "redo" } else { "undo" }));
    }
    ToolReply::Text(format!("{verb}: {}", done.join(", ")))
}
