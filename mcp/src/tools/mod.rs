//! The tools themselves, and the pump that runs them.
//!
//! Every function here is called from an exclusive system, so it holds
//! `&mut World` outright and needs no locking, no deferred commands and no
//! thought about what else might be running. That is the payoff for the channel
//! hop in [`crate::rpc`], and the reason a new tool is usually twenty lines.
//!
//! Output is prose, not JSON. A model reads these replies, and a scene tree it
//! can skim beats a nest of objects it has to walk; ids are the one thing that
//! must survive a round trip verbatim, so those are printed exactly as they will
//! be accepted back.
//!
//! Split by what a tool does: [`observe`] looks without changing anything,
//! [`scene`] makes and moves entities, [`components`] edits what is on them, and
//! [`editor`] drives the editor itself. The content tools (materials, models,
//! particles, UI) live in [`crate::content`].

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::reflect::{TypeRegistration, TypeRegistry};
use renzora::core::console_log::console_warn;
use renzora::serde_json::Value;
use renzora::PropertyValue;
use std::time::Instant;

use crate::{content, McpBridge, ToolCall, ToolReply};

mod components;
mod editor;
mod observe;
mod scene;

pub use observe::read_complete_png;
pub(crate) use scene::material_handle;
pub use scene::AssetCache;

/// Drain the queue and run whatever is in it.
///
/// Everything is taken out of the channel before the first tool runs: a tool
/// needs `&mut World`, and the resource holding the receiver lives in that same
/// world, so the borrow has to be finished before dispatch starts.
pub fn pump(world: &mut World) {
    let calls: Vec<ToolCall> = {
        let Some(bridge) = world.get_resource::<McpBridge>() else {
            return;
        };
        bridge.state.mark_pump();
        let Ok(rx) = bridge.rx.lock() else {
            return;
        };
        rx.try_iter().collect()
    };

    for call in calls {
        dispatch(world, call);
    }
}

fn dispatch(world: &mut World, call: ToolCall) {
    let ToolCall {
        name,
        mut args,
        reply,
        expires,
    } = call;

    if Instant::now() >= expires {
        console_warn(
            crate::CATEGORY,
            format!("dropped {name}: its caller had already stopped waiting"),
        );
        return;
    }

    normalize_entity_ids(world, &mut args);

    // Screenshots and imports answer a frame or more later, so these arms hand
    // the reply channel over instead of using it.
    if name == "screenshot" {
        observe::start_screenshot(world, reply);
        return;
    }
    if name == "import_model" {
        content::start_import(world, &args, reply);
        return;
    }

    let outcome = match name.as_str() {
        "editor_state" => observe::editor_state(world),
        "read_console" => observe::read_console(&args),
        "scene_tree" => observe::scene_tree(world, &args),
        "inspect_entity" => observe::inspect_entity(world, &args),
        "spawn_entity" => scene::spawn_entity(world, &args),
        "spawn_light" => scene::spawn_light(world, &args),
        "list_shapes" => scene::list_shapes(world),
        "make_solid" => scene::make_solid(world, &args),
        "rename_entity" => scene::rename_entity(world, &args),
        "reparent_entity" => scene::reparent_entity(world, &args),
        "duplicate_entity" => scene::duplicate_entity(world, &args),
        "despawn_entity" => scene::despawn_entity(world, &args),
        "set_transform" => scene::set_transform(world, &args),
        "add_component" => components::add_component(world, &args),
        "remove_component" => components::remove_component(world, &args),
        "set_component_field" => components::set_component_field(world, &args),
        "attach_script" => components::attach_script(world, &args),
        "detach_script" => components::detach_script(world, &args),
        "set_view" => editor::set_view(world, &args),
        "editor_request" => editor::editor_request(world, &args),
        "select_entities" => editor::select_entities(world, &args),
        "set_play_mode" => editor::set_play_mode(world, &args),
        "save_scene" => editor::save_scene(world),
        "undo" => editor::undo_redo(world, &args),
        "set_autosave" => editor::set_autosave(world, &args),
        "list_assets" => content::list_assets(world, &args),
        "set_material" => content::set_material(world, &args),
        "spawn_model" => content::spawn_model(world, &args),
        "spawn_particles" => content::spawn_particles(world, &args),
        "add_ui" => content::add_ui(world, &args),
        "list_presets" => content::list_presets(world),
        "spawn_preset" => content::spawn_preset(world, &args),
        "set_default_camera" => content::set_default_camera(world, &args),
        other => ToolReply::Failed(format!("no such tool: {other}")),
    };

    let _ = reply.send(outcome);
}

// ============================================================================
// Argument and value plumbing
// ============================================================================

/// How an entity is written on the wire.
///
/// A decimal string of `Entity::to_bits`, and a string rather than a number on
/// purpose: those bits pack a generation into the high 32, so an entity from a
/// long session exceeds the 2^53 a JSON number carries exactly, and the id would
/// come back rounded to a different entity.
pub(crate) fn entity_id(entity: Entity) -> String {
    entity.to_bits().to_string()
}

/// Rewrite every `2720v0`-style id in the arguments into the form the tools
/// take.
///
/// That form is how Bevy prints an entity, so it is what the Console and the
/// panics show, and an agent reading either has no other way to name the entity
/// it just saw. The lookup is a scan of every live entity for one that prints
/// the same, which is simpler than rebuilding an `Entity` from its parts and
/// cannot disagree with Bevy about the layout of those parts.
fn normalize_entity_ids(world: &mut World, args: &mut Value) {
    fn looks_like_display(text: &str) -> bool {
        text.split_once('v').is_some_and(|(index, generation)| {
            !index.is_empty()
                && !generation.is_empty()
                && index.bytes().all(|b| b.is_ascii_digit())
                && generation.bytes().all(|b| b.is_ascii_digit())
        })
    }
    fn collect<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
        match value {
            Value::String(text) if looks_like_display(text.trim()) => out.push(text.trim()),
            Value::Array(items) => items.iter().for_each(|v| collect(v, out)),
            Value::Object(map) => map.values().for_each(|v| collect(v, out)),
            _ => {}
        }
    }
    fn rewrite(value: &mut Value, found: &HashMap<String, String>) {
        match value {
            Value::String(text) => {
                if let Some(bits) = found.get(text.trim()) {
                    *text = bits.clone();
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|v| rewrite(v, found)),
            Value::Object(map) => map.values_mut().for_each(|v| rewrite(v, found)),
            _ => {}
        }
    }

    let mut wanted = Vec::new();
    collect(args, &mut wanted);
    if wanted.is_empty() {
        return;
    }
    let wanted: Vec<String> = wanted.into_iter().map(str::to_string).collect();
    let mut found: HashMap<String, String> = HashMap::default();
    let mut all = world.query::<Entity>();
    for entity in all.iter(world) {
        let shown = entity.to_string();
        if wanted.contains(&shown) {
            found.insert(shown, entity_id(entity));
        }
    }
    rewrite(args, &found);
}

pub(crate) fn arg_entity(args: &Value, key: &str) -> Option<Entity> {
    value_to_entity(args.get(key)?)
}

/// Accepts the string form this server hands out, and a plain number as well,
/// since a client that has been round-tripping ids through JSON may have turned
/// one into a number on the way.
fn value_to_entity(value: &Value) -> Option<Entity> {
    let bits = match value {
        Value::String(text) => text.trim().parse::<u64>().ok()?,
        Value::Number(number) => number.as_u64()?,
        _ => return None,
    };
    Entity::try_from_bits(bits)
}

pub(crate) fn arg_vec3(args: &Value, key: &str) -> Option<Vec3> {
    let array = args.get(key)?.as_array()?;
    if array.len() < 3 {
        return None;
    }
    Some(Vec3::new(
        array[0].as_f64()? as f32,
        array[1].as_f64()? as f32,
        array[2].as_f64()? as f32,
    ))
}

/// Turn a JSON value into the contract crate's property vocabulary, which is
/// what the reflection setter speaks.
fn property_value(value: &Value) -> Option<PropertyValue> {
    match value {
        Value::Bool(b) => Some(PropertyValue::Bool(*b)),
        Value::String(s) => Some(PropertyValue::String(s.clone())),
        Value::Number(n) => Some(PropertyValue::Float(n.as_f64()? as f32)),
        Value::Array(items) => {
            let numbers: Option<Vec<f32>> = items
                .iter()
                .map(|item| item.as_f64().map(|f| f as f32))
                .collect();
            let numbers = numbers?;
            match numbers.len() {
                3 => Some(PropertyValue::Vec3([numbers[0], numbers[1], numbers[2]])),
                4 => Some(PropertyValue::Color([
                    numbers[0], numbers[1], numbers[2], numbers[3],
                ])),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Look a type up the way a person names it.
///
/// Short path first, because `Transform` is what anybody types; full path next,
/// for the case where two crates both define a `Settings`; and a case-insensitive
/// sweep last, so `pointlight` finds `PointLight` rather than reporting that no
/// such component exists.
///
/// The sweep also catches a short name the registry calls ambiguous, which it
/// answers with nothing. Components come first there: every caller wants one,
/// and `Rotation` names both avian's component and a plain type that is not.
pub(crate) fn find_type<'a>(registry: &'a TypeRegistry, name: &str) -> Option<&'a TypeRegistration> {
    use bevy::ecs::reflect::ReflectComponent;
    let named = |registration: &&TypeRegistration| {
        registration
            .type_info()
            .type_path_table()
            .short_path()
            .eq_ignore_ascii_case(name)
    };
    registry
        .get_with_short_type_path(name)
        .or_else(|| registry.get_with_type_path(name))
        .or_else(|| {
            registry
                .iter()
                .filter(named)
                .find(|r| r.data::<ReflectComponent>().is_some())
        })
        .or_else(|| registry.iter().find(named))
}
