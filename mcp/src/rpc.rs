//! The MCP envelope: JSON-RPC 2.0 framing, the handshake, the tool catalogue,
//! and the hop onto the main thread that a `tools/call` turns into.
//!
//! Everything here runs on a socket thread and touches no Bevy state. The one
//! thing it knows about the engine is that a tool call has to be answered by
//! somebody else: it posts a [`ToolCall`] and blocks on the reply channel until
//! the pump in [`crate::tools`] gets to it.

use renzora::serde_json::{json, Value};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::{ServerState, ToolCall, ToolReply};

/// The version replied to a client that names none. MCP negotiates by having the
/// client state a version and the server answer with one it can speak, so the
/// client's own version is echoed when it sends one: it is by definition a
/// version that client understands, and every message this server sends fits
/// every revision of the protocol that has existed.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// This plugin's version, as the handshake reports it. Keep it equal to
/// `version` in `Cargo.toml`.
///
/// Written out rather than `env!("CARGO_PKG_VERSION")`, because the editor
/// builds a native plugin by calling rustc directly and cargo is what sets that
/// variable. It compiled here only while the editor had been started through
/// cargo and inherited it; launched any other way, the plugin failed to build.
const SERVER_VERSION: &str = "1.0.0";

/// How long a tool may take before the caller is told it did not finish.
///
/// Generous because the pump runs once per frame and a frame can be long while
/// the editor is loading a project, so a tight timeout would report failure for
/// work that was merely queued behind a slow frame.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// Screenshots get longer: the capture is resolved by the render app, and the
/// first one after launch can wait on a pipeline that is still compiling.
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(45);

/// Imports get the longest: converting a large FBX, its textures and its
/// materials is real work, done inside one engine frame.
const IMPORT_TIMEOUT: Duration = Duration::from_secs(120);

/// How long without a frame before a call is refused outright as frozen.
/// Comfortably longer than the slowest real frame, a project load, so a busy
/// editor is waited on and only a stuck one is turned away.
const FROZEN_AFTER: Duration = Duration::from_secs(10);

/// Handle one request body. `None` means "nothing to send back", which is the
/// correct answer to a notification and to a batch made only of notifications.
pub fn handle(body: &str, tx: &Sender<ToolCall>, state: &ServerState) -> Option<String> {
    let parsed: Value = match renzora::serde_json::from_str(body) {
        Ok(value) => value,
        Err(err) => {
            return Some(
                error_response(Value::Null, -32700, &format!("invalid JSON: {err}")).to_string(),
            )
        }
    };

    // A batch is answered by a batch of only the messages that have replies. A
    // batch of pure notifications therefore answers with nothing at all, rather
    // than with an empty array, which the spec forbids.
    if let Some(batch) = parsed.as_array() {
        let replies: Vec<Value> = batch
            .iter()
            .filter_map(|msg| one(msg, tx, state))
            .collect();
        return (!replies.is_empty()).then(|| Value::Array(replies).to_string());
    }

    one(&parsed, tx, state).map(|reply| reply.to_string())
}

/// Handle a single JSON-RPC message.
fn one(msg: &Value, tx: &Sender<ToolCall>, state: &ServerState) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = msg.get("id").cloned();

    // No id means a notification: the client is telling, not asking, and the
    // spec says a response to one is an error. `notifications/initialized`
    // arrives on every connection and is the common case.
    let Some(id) = id else {
        return None;
    };

    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION)
                .to_string();
            Some(result_response(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "renzora", "version": SERVER_VERSION },
                    "instructions": INSTRUCTIONS,
                }),
            ))
        }
        "ping" => Some(result_response(id, json!({}))),
        "tools/list" => Some(result_response(id, json!({ "tools": catalogue() }))),
        "tools/call" => Some(call(id, &params, tx, state)),
        // Declared in no capability, so a well-behaved client never asks. Answered
        // anyway, and with an empty list rather than an error, because a client
        // that probes on connect should not have its handshake fail over a
        // feature this server simply does not have.
        "resources/list" => Some(result_response(id, json!({ "resources": [] }))),
        "prompts/list" => Some(result_response(id, json!({ "prompts": [] }))),
        _ => Some(error_response(
            id,
            -32601,
            &format!("unknown method: {method}"),
        )),
    }
}

/// Post a tool call to the main thread and wait for its answer.
fn call(id: Value, params: &Value, tx: &Sender<ToolCall>, state: &ServerState) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_response(id, -32602, "a tools/call needs a name");
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let timeout = match name {
        "screenshot" => SCREENSHOT_TIMEOUT,
        "import_model" => IMPORT_TIMEOUT,
        _ => CALL_TIMEOUT,
    };

    // A main thread that has not run a frame in this long is not going to answer
    // inside the timeout either. Saying so now, and queueing nothing, beats a
    // twenty-second wait for a generic failure and a call that would run whenever
    // the editor came back.
    if let Some(stalled) = state.since_pump().filter(|d| *d >= FROZEN_AFTER) {
        return tool_error(
            id,
            &format!(
                "the editor has not run a frame for {}s: it is frozen, minimised, or loading something very large. Nothing was done; check the editor window before retrying",
                stalled.as_secs()
            ),
        );
    }

    let (reply_tx, reply_rx) = std::sync::mpsc::channel::<ToolReply>();
    if tx
        .send(ToolCall {
            name: name.to_string(),
            args,
            reply: reply_tx,
            expires: Instant::now() + timeout,
        })
        .is_err()
    {
        // The receiving end only drops when the App is being torn down, so this
        // is the editor shutting under the client rather than a bad request.
        return tool_error(id, "the editor is no longer accepting calls");
    }
    state.call_made();

    match reply_rx.recv_timeout(timeout) {
        Ok(ToolReply::Text(text)) => result_response(
            id,
            json!({ "content": [ { "type": "text", "text": text } ], "isError": false }),
        ),
        Ok(ToolReply::Image { data, mime }) => result_response(
            id,
            json!({
                "content": [ { "type": "image", "data": data, "mimeType": mime } ],
                "isError": false
            }),
        ),
        Ok(ToolReply::Failed(why)) => tool_error(id, &why),
        Err(_) => tool_error(
            id,
            "the editor did not answer in time; it may be busy or minimised",
        ),
    }
}

/// A failed tool is reported inside a successful JSON-RPC result, with
/// `isError`. That is MCP's rule and it matters: a protocol-level error is the
/// client's problem to handle, while a tool that could not do its job is
/// something the model should read and react to.
fn tool_error(id: Value, message: &str) -> Value {
    result_response(
        id,
        json!({ "content": [ { "type": "text", "text": message } ], "isError": true }),
    )
}

fn result_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Shown to the model once, at handshake. Worth the words: the two things it
/// says are both invisible from the tool list and both lead to wasted calls.
const INSTRUCTIONS: &str = "\
These tools drive a running Renzora editor. Entity ids are opaque strings; pass \
back exactly what a tool gave you. An id as the Console prints it, such as 2720v0, \
is accepted too. Every edit goes on the editor's undo stack, so the user can take \
it back with Ctrl+Z, except despawn_entity. Content is file-based: a material, \
model, particle effect or UI template is a file in the project, and a path outside \
the project is copied in first. list_assets shows what the project has, and with \
source engine the engine's own presets. Save with save_scene when the user is \
happy; nothing is written to the scene file until then.";

/// An entity id, described once so every tool that takes one agrees.
fn entity_arg(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

/// What a tool does to the world, as MCP's behaviour hints.
///
/// Worth filling in rather than leaving off: without them a client cannot tell
/// `read_console` from `despawn_entity`, so it must either confirm everything or
/// confirm nothing. `readOnlyHint` is the one that earns its keep, because it is
/// what lets an agent look freely and stop to ask before it changes something.
///
/// `destructiveHint` marks the tools whose effect cannot be walked back. Every
/// edit here goes on the editor's undo stack (see `edits.rs`) except a despawn,
/// so a despawned entity really is gone.
fn hints(read_only: bool, destructive: bool) -> Value {
    json!({
        "readOnlyHint": read_only,
        "destructiveHint": destructive,
        // Nothing here is safe to retry blindly: spawning twice makes two
        // entities, and a request fired twice is asked for twice.
        "idempotentHint": false,
        // The editor is a world this server does not own the whole of; a person
        // is editing it at the same time.
        "openWorldHint": true
    })
}

fn vec3_arg(description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": "number" },
        "minItems": 3,
        "maxItems": 3,
        "description": description
    })
}

/// The tool catalogue, in the order a session tends to need it: look, then
/// change, then look again.
fn catalogue() -> Value {
    let mut tools = json!([
        {
            "name": "editor_state",
            "description": "What the editor is doing right now: open project, current scene, play mode, selection, entity count and frame rate. Start here.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "read_console",
            "description": "Recent lines from the editor's Console panel, newest last. This is where compile errors, script errors and plugin logs appear.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "How many lines to return (default 50)." },
                    "level": { "type": "string", "enum": ["info", "success", "warning", "error"], "description": "Only lines at this level." },
                    "contains": { "type": "string", "description": "Only lines whose message or category contains this text, case-insensitively." }
                }
            }
        },
        {
            "name": "scene_tree",
            "description": "The scene's named entities as a tree, with their ids. Editor chrome is left out unless include_hidden is set.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "include_hidden": { "type": "boolean", "description": "Include entities the hierarchy panel hides (gizmos, preview rigs, UI)." },
                    "max_depth": { "type": "integer", "description": "How deep to descend (default 8)." }
                }
            }
        },
        {
            "name": "inspect_entity",
            "description": "One entity's components and their reflected field values.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity id, as given by scene_tree or editor_state."),
                    "component": { "type": "string", "description": "Only this component, by short name such as Transform. Omit for all of them." }
                },
                "required": ["entity"]
            }
        },
        {
            "name": "screenshot",
            "description": "A PNG of the editor window as it looks now, including the panels and the viewport.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "spawn_entity",
            "description": "Spawn a named entity, optionally with a primitive mesh and a colour. Returns its id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "The name shown in the hierarchy. Required: an unnamed entity is not saved with the scene." },
                    "shape": { "type": "string", "description": "A shape id from list_shapes, such as cube, stairs, arch or ramp. Omit for an empty transform, which is what a group wants." },
                    "position": vec3_arg("World position (default 0, 0, 0)."),
                    "rotation": vec3_arg("Euler XYZ rotation in degrees (default 0, 0, 0)."),
                    "scale": vec3_arg("Scale (default 1, 1, 1)."),
                    "color": vec3_arg("Base colour as LINEAR RGB 0..1, when a shape is given. Linear means pale values wash out to near white under daylight: a mid grey is about 0.2, not 0.5. For anything beyond a flat colour, use set_material."),
                    "collider": { "type": "boolean", "description": "Give it a static collider so things can stand on it. On by default for any shape; set false for pure decoration." },
                    "parent": entity_arg("Parent to attach to. Omit to spawn at the scene root.")
                },
                "required": ["name"]
            }
        },
        {
            "name": "list_shapes",
            "description": "Every shape spawn_entity can build, by category. Read this before building geometry: the list is the project's, not a fixed one, and it holds level pieces such as stairs, ramps, arches and pillars as well as the basic solids.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "add_component",
            "description": "Add any registered component by name and set its fields. This reaches components no other tool here names, including ones from the user's own game.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity to add it to."),
                    "component": { "type": "string", "description": "Short name, such as PointLight, Collider or your own component's type name." },
                    "fields": { "type": "object", "description": "Field paths to values, applied after the component is added. Same value rules as set_component_field." }
                },
                "required": ["entity", "component"]
            }
        },
        {
            "name": "remove_component",
            "description": "Remove a component from an entity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity to edit."),
                    "component": { "type": "string", "description": "Short name of the component to remove." }
                },
                "required": ["entity", "component"]
            }
        },
        {
            "name": "make_solid",
            "description": "Give colliders to an entity and every mesh under it that has not got one. For geometry built before colliders, or spawned as decoration and wanted solid after all.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The root of the subtree to make solid."),
                    "shape": { "type": "string", "description": "Fallback collider shape, used only for a mesh that does not record what shape it is (default cube). Anything spawned by spawn_entity records it and is matched exactly." }
                },
                "required": ["entity"]
            }
        },
        {
            "name": "set_view",
            "description": "Move the editor's viewport camera. Writing the camera's Transform does nothing; this is the supported way, and it is what the View menu and the F key use.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "eye": vec3_arg("Where to put the camera."),
                    "target": vec3_arg("What it looks at. The orbit pivot lands here, so the user's next drag is about this point."),
                    "frame": entity_arg("Instead of eye and target: frame this entity and its descendants, as the F key does."),
                    "frame_all": { "type": "boolean", "description": "Instead of either: fit the whole scene in view." }
                }
            }
        },
        {
            "name": "spawn_light",
            "description": "Spawn a light. A scene built only from spawn_entity renders black, so this is usually the second call after the first mesh.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "The name shown in the hierarchy." },
                    "kind": { "type": "string", "enum": ["directional", "point", "spot"], "description": "directional is a sun and lights everything; point and spot are local (default directional)." },
                    "position": vec3_arg("World position (default 0, 8, 0). Ignored in effect by a directional light, which only cares about its rotation."),
                    "rotation": vec3_arg("Euler XYZ rotation in degrees. A directional light defaults to an afternoon sun angle; leaving it at zero would point it straight along -Z and light nothing."),
                    "color": vec3_arg("Linear RGB 0..1 (default white)."),
                    "illuminance": { "type": "number", "description": "Directional only, in lux. Default 10000, which is overcast daylight." },
                    "intensity": { "type": "number", "description": "Point and spot, in lumens. Default 1000000." },
                    "range": { "type": "number", "description": "Point and spot, in world units. Default 20." },
                    "inner_angle": { "type": "number", "description": "Spot only, degrees. Default 20." },
                    "outer_angle": { "type": "number", "description": "Spot only, degrees. Default 35." },
                    "parent": entity_arg("Parent to attach to.")
                },
                "required": ["name"]
            }
        },
        {
            "name": "despawn_entity",
            "description": "Despawn an entity and its children.",
            "inputSchema": {
                "type": "object",
                "properties": { "entity": entity_arg("The entity to remove.") },
                "required": ["entity"]
            }
        },
        {
            "name": "set_transform",
            "description": "Move, rotate or scale an entity. Omitted parts are left alone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity to move."),
                    "position": vec3_arg("New world position."),
                    "rotation": vec3_arg("New Euler XYZ rotation in degrees."),
                    "scale": vec3_arg("New scale.")
                },
                "required": ["entity"]
            }
        },
        {
            "name": "set_component_field",
            "description": "Write one reflected field of one component, by dotted path such as translation.x or base_color.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity to edit."),
                    "component": { "type": "string", "description": "Component short name, such as Transform or PointLight." },
                    "field": { "type": "string", "description": "Dotted field path within the component." },
                    "value": { "description": "Number, boolean, string, or an array of 3 or 4 numbers for a vector or colour." }
                },
                "required": ["entity", "component", "field", "value"]
            }
        },
        {
            "name": "select_entities",
            "description": "Set the editor's selection, which is what the inspector and the gizmos follow.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entities": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "The entities to select. An empty array clears the selection."
                    }
                },
                "required": ["entities"]
            }
        },
        {
            "name": "set_play_mode",
            "description": "Start, pause, stop or simulate. The request is honoured on the editor's next frame.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["play", "pause", "stop", "simulate"], "description": "play and stop enter and leave play mode; simulate runs the scene with the editor still live; pause toggles." }
                },
                "required": ["mode"]
            }
        },
        {
            "name": "save_scene",
            "description": "Ask the editor to save the open scene, the way Ctrl+S does.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "editor_request",
            "description": "Ask the editor for one of the things its menus ask for: new/open/save-as scene, export, import, settings, the command palette, isolation mode. Some open a modal file dialog and then wait for a person; the reply says which.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "request": { "type": "string", "enum": ["save_scene", "save_scene_as", "new_scene", "open_scene", "open_scene_path", "open_code_file", "open_ui_template", "export", "import_files", "import_folder", "check_updates", "tutorial", "settings", "create_node", "command_palette", "isolation"], "description": "Which request to make. Prefer open_scene_path over open_scene: the latter opens a dialog a person has to answer." },
                    "path": { "type": "string", "description": "For open_scene_path, open_code_file and open_ui_template." },
                    "active": { "type": "boolean", "description": "For isolation. Omit to toggle." }
                },
                "required": ["request"]
            }
        },
        {
            "name": "rename_entity",
            "description": "Change an entity's name, which is what the hierarchy shows and what a saved scene keys on.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity to rename."),
                    "name": { "type": "string", "description": "The new name." }
                },
                "required": ["entity", "name"]
            }
        },
        {
            "name": "reparent_entity",
            "description": "Move an entity under a different parent, or out to the scene root. Its transform stays a local one, so it moves with the new parent.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity to move."),
                    "parent": entity_arg("The new parent. Omit to move it out to the scene root.")
                },
                "required": ["entity"]
            }
        },
        {
            "name": "duplicate_entity",
            "description": "Copy an entity with all its components and its whole subtree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity to copy."),
                    "name": { "type": "string", "description": "Name for the copy. Omit to keep the original's, which leaves two entities sharing a name." },
                    "offset": vec3_arg("Move the copy by this much, so it does not sit exactly inside the original.")
                },
                "required": ["entity"]
            }
        },
        {
            "name": "attach_script",
            "description": "Attach a script file (.rs, or any language a backend is installed for) to an entity. Write the file into the project first; a .rs compiles on save and its result appears in read_console. Calling again on an attached script just changes preview.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity to attach it to."),
                    "path": { "type": "string", "description": "The script, relative to the project root (scripts/spin.rs) or absolute inside it." },
                    "preview": { "type": "boolean", "description": "Run it now in edit mode, like the inspector's per-script play button, without entering play mode. Off by default." }
                },
                "required": ["entity", "path"]
            }
        },
        {
            "name": "detach_script",
            "description": "Remove a script from an entity. The file itself is left alone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity to remove it from."),
                    "path": { "type": "string", "description": "The script path, as given to attach_script." }
                },
                "required": ["entity", "path"]
            }
        }
    ]);
    // Two literals rather than one: a single `json!` this long runs past the
    // macro's recursion limit, and splitting it keeps the fix out of the crate
    // attributes and lets the list keep growing.
    let more = json!([
        {
            "name": "undo",
            "description": "Step the scene's undo history back, as Ctrl+Z does, or forward with redo. This is the user's history as well as yours, so the reply names each edit it reversed; check it went no further than you meant.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": { "type": "integer", "description": "How many edits (default 1, at most 50)." },
                    "redo": { "type": "boolean", "description": "Go forward instead, as Ctrl+Y does." }
                }
            }
        },
        {
            "name": "set_autosave",
            "description": "Pause or resume the editor's autosave for this session only; the user's saved preference is untouched. Pause it before experimenting, so test edits are not written into a real scene file, and so an unsaved tab does not pop a blocking Save As dialog. Omit enabled to read the current state.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean", "description": "true to resume, false to pause." }
                }
            }
        },
        {
            "name": "list_assets",
            "description": "Files in the open project by kind, as the paths other tools take. With source engine, the engine's own presets instead (particle effects, UI templates, materials); pass one of those paths to a tool and it is copied into the project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["all", "materials", "models", "particles", "ui", "scripts", "scenes", "textures", "audio"], "description": "Which files (default all)." },
                    "source": { "type": "string", "enum": ["project", "engine"], "description": "project (default), or engine for the presets; engine presets exist only when the editor runs from an engine checkout." },
                    "contains": { "type": "string", "description": "Only paths containing this text, case-insensitively." },
                    "limit": { "type": "integer", "description": "Most paths to return (default 200)." }
                }
            }
        },
        {
            "name": "set_material",
            "description": "Give an entity's meshes a material. Either name an existing .material file, or give colours and a new .material is written into the project's materials folder. Applies to every mesh under the entity by default, so it works on an imported model as well as a shape.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": entity_arg("The entity whose meshes get the material."),
                    "path": { "type": "string", "description": "An existing .material, project-relative or absolute. Omit to write a new one from the fields below." },
                    "name": { "type": "string", "description": "File name for a new material (default material). A number is added if taken." },
                    "base_color": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 4, "description": "Linear RGB or RGBA 0..1. Alpha below 1 makes it transparent." },
                    "metallic": { "type": "number", "description": "0..1 (default 0)." },
                    "roughness": { "type": "number", "description": "0..1 (default 0.5)." },
                    "emissive": vec3_arg("Linear RGB glow, in physical units: under the default daylight sky it needs values in the thousands to show (e.g. [0, 2000, 6000] for a bright blue); single digits only read at night or indoors."),
                    "double_sided": { "type": "boolean", "description": "Render back faces too (foliage, glass, cloth)." },
                    "recursive": { "type": "boolean", "description": "Include meshes under the entity (default true)." }
                },
                "required": ["entity"]
            }
        },
        {
            "name": "import_model",
            "description": "Import a 3D model (glb, gltf, fbx, obj, stl, ply, dae, usd, abc) into the project. A file outside the project is copied into models/<name>/ first; the engine converts it to .glb with its textures and materials. Answers when done, with the .glb path to pass to spawn_model. Large files can take a while.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The model file, absolute or project-relative. One inside the project is converted where it stands, and a non-GLB source is then replaced by the .glb." },
                    "folder": { "type": "string", "description": "Project folder to copy into (default models)." }
                },
                "required": ["path"]
            }
        },
        {
            "name": "spawn_model",
            "description": "Place an imported .glb in the scene. The model loads over the next frames and its meshes appear as children.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The .glb, as import_model or list_assets gave it." },
                    "name": { "type": "string", "description": "Name in the hierarchy (default the file name)." },
                    "position": vec3_arg("World position (default origin)."),
                    "rotation": vec3_arg("Euler XYZ rotation in degrees."),
                    "scale": vec3_arg("Scale (default 1)."),
                    "parent": entity_arg("Parent to attach to.")
                },
                "required": ["path"]
            }
        },
        {
            "name": "spawn_particles",
            "description": "Spawn a particle effect from a .particle file. list_assets kind particles (source engine for the presets, such as fire, smoke, sparks, rain) shows what exists.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The .particle file: project-relative, absolute, or an engine preset path." },
                    "name": { "type": "string", "description": "Name in the hierarchy (default the file name)." },
                    "position": vec3_arg("World position (default origin)."),
                    "rotation": vec3_arg("Euler XYZ rotation in degrees."),
                    "size": { "type": "number", "description": "Size of the effect (default 1). Presets are authored at different sizes, so a fire or firefly preset may want 0.2 to 0.5 beside human-scale props; the entity's transform scale does not resize particles." },
                    "rate": { "type": "number", "description": "Spawn-rate multiplier (default 1). Lower it to thin out a dense effect and save frame time." },
                    "time_scale": { "type": "number", "description": "Playback speed (default 1)." },
                    "parent": entity_arg("Parent to attach to, so the effect follows it.")
                },
                "required": ["path"]
            }
        },
        {
            "name": "add_ui",
            "description": "Show an .html UI template (HUD, menu, health bar, dialog) on a screen canvas of its own. list_assets kind ui (source engine for the templates) shows what exists. Edit the .html file itself to change the UI.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "template": { "type": "string", "description": "The .html template: project-relative, absolute, or an engine template path." },
                    "name": { "type": "string", "description": "Name for the canvas (default the template's name)." }
                },
                "required": ["template"]
            }
        },
        {
            "name": "list_presets",
            "description": "Everything the editor's Add Entity menu can spawn: cameras, lights, terrain, world environment, reflection probes, colliders, UI canvas, and whatever plugins add.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "spawn_preset",
            "description": "Spawn an Add Entity preset by id, exactly as the menu would. Use camera_3d for a game camera, world_environment for sky and lighting, terrain for ground.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The preset id from list_presets." },
                    "name": { "type": "string", "description": "Rename it (default the preset's own name)." },
                    "position": vec3_arg("World position, where the preset has a transform."),
                    "parent": entity_arg("Parent to attach to.")
                },
                "required": ["id"]
            }
        },
        {
            "name": "set_default_camera",
            "description": "Make a scene camera the one the game looks through in play mode. To aim it, use set_transform on the camera; set_view moves the editor's own camera, not this one.",
            "inputSchema": {
                "type": "object",
                "properties": { "entity": entity_arg("The scene camera.") },
                "required": ["entity"]
            }
        }
    ]);
    if let (Some(list), Value::Array(more)) = (tools.as_array_mut(), more) {
        list.extend(more);
    }
    annotate(&mut tools);
    tools
}

/// Attach behaviour hints to every tool.
///
/// Here rather than beside each entry so the read/write split is one list that
/// can be read at a glance and audited. A tool missing from the write list is
/// treated as read-only, so the failure mode of forgetting one is a client
/// trusting a tool it should have questioned: the match is therefore exhaustive
/// over the writers, and anything new lands in `_` only by being named.
fn annotate(tools: &mut Value) {
    let Some(entries) = tools.as_array_mut() else {
        return;
    };
    for tool in entries {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let (read_only, destructive) = match name.as_str() {
            // Looking, in every case.
            "editor_state" | "read_console" | "scene_tree" | "inspect_entity" | "screenshot"
            | "list_shapes" | "list_assets" | "list_presets" => (true, false),
            // Gone for good: a despawn does not reach the editor's undo stack.
            "despawn_entity" => (false, true),
            // Changes the world, recoverably by doing something else.
            _ => (false, false),
        };
        tool["annotations"] = hints(read_only, destructive);
    }
}
