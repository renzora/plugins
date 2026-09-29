//! Tools that edit what is on an entity: any registered component by name, one
//! reflected field, and the script list.

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::std_traits::ReflectDefault;
use renzora::component::ScriptComponent;
use renzora::core::reflection::{insert_component_reflected, set_reflected_field};
use renzora::serde_json::Value;
use renzora::{CurrentProject, PropertyValue};
use std::path::{Path, PathBuf};

use super::{arg_entity, entity_id, find_type, property_value};
use crate::{edits, ToolReply};

/// Add any registered component by name, then set whichever of its fields the
/// caller named.
///
/// This is the tool that stops the catalogue needing a new entry per component
/// type. A plugin can only link three crates, so `Collider` and `DirectionalLight`
/// are reachable only because the contract crate re-exports them: everything
/// else in the engine, and everything in the user's own game, is reachable ONLY
/// through the type registry. Going through reflection means a component this
/// plugin has never heard of works the day it is registered.
///
/// The limit is honest and worth knowing: a type with no `#[reflect(Default)]`
/// cannot be built from nothing, and this says so by name rather than failing
/// vaguely.
pub(super) fn add_component(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("add_component needs an entity id".into());
    };
    let Some(wanted) = args.get("component").and_then(Value::as_str) else {
        return ToolReply::Failed("add_component needs a component name".into());
    };
    if world.get_entity(entity).is_err() {
        return ToolReply::Failed(format!("{} is not a live entity", entity_id(entity)));
    }

    let registry = world.resource::<AppTypeRegistry>().clone();
    let (short_path, full_path, value) = {
        let reg = registry.read();
        let Some(registration) = find_type(&reg, wanted) else {
            return ToolReply::Failed(format!(
                "no registered component called {wanted}; names are short ones such as Transform or PointLight"
            ));
        };
        let short = registration.type_info().type_path_table().short_path();
        if registration.data::<ReflectComponent>().is_none() {
            return ToolReply::Failed(format!("{short} is a registered type but not a component"));
        }
        let Some(default) = registration.data::<ReflectDefault>() else {
            return ToolReply::Failed(format!(
                "{short} has no #[reflect(Default)], so it cannot be created from nothing. Spawn it with a dedicated tool, or set its fields on an entity that already has one"
            ));
        };
        (
            short.to_string(),
            registration.type_info().type_path().to_string(),
            default.default(),
        )
    };

    // Taken before the insert, which replaces a component the entity already had
    // with a default one; undo has to give the old value back, not remove it.
    let before = edits::snapshot_component(world, entity, &full_path);

    if !insert_component_reflected(world, entity, &short_path, value.as_ref()) {
        return ToolReply::Failed(format!("could not insert {short_path}"));
    }

    // Fields after insertion rather than onto the default value, so each one goes
    // through the same coercion path `set_component_field` uses and a field that
    // does not fit is reported by name instead of silently doing nothing.
    let mut set = Vec::new();
    let mut missed = Vec::new();
    if let Some(fields) = args.get("fields").and_then(Value::as_object) {
        for (field, raw) in fields {
            match property_value(raw) {
                Some(value) if set_reflected_field(world, entity, &short_path, field, &value) => {
                    set.push(field.clone())
                }
                _ => missed.push(field.clone()),
            }
        }
    }

    // After the fields, so one Ctrl+Z takes back the component and its values
    // together rather than stepping through them.
    let after = edits::snapshot_component(world, entity, &full_path);
    edits::record_component(
        world,
        entity,
        &full_path,
        format!("add {short_path}"),
        before,
        after,
    );

    let mut out = format!("added {short_path} to {}", entity_id(entity));
    if !set.is_empty() {
        out.push_str(&format!(", set {}", set.join(", ")));
    }
    if !missed.is_empty() {
        out.push_str(&format!(
            " (could not set {}: no such field, or the value does not fit)",
            missed.join(", ")
        ));
    }
    ToolReply::Text(out)
}

pub(super) fn remove_component(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("remove_component needs an entity id".into());
    };
    let Some(wanted) = args.get("component").and_then(Value::as_str) else {
        return ToolReply::Failed("remove_component needs a component name".into());
    };

    let registry = world.resource::<AppTypeRegistry>().clone();
    let (short, full_path, removed) = {
        let reg = registry.read();
        let Some(registration) = find_type(&reg, wanted) else {
            return ToolReply::Failed(format!("no registered component called {wanted}"));
        };
        let short = registration.type_info().type_path_table().short_path().to_string();
        let Some(reflect_component) = registration.data::<ReflectComponent>() else {
            return ToolReply::Failed(format!("{short} is a registered type but not a component"));
        };
        let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
            return ToolReply::Failed(format!("{} is not a live entity", entity_id(entity)));
        };
        let removed = reflect_component.take(&mut entity_mut);
        (short, registration.type_info().type_path().to_string(), removed)
    };

    // `take` hands back the value it removed, which is exactly what undo needs
    // to put back. Nothing removed means nothing to undo, and no stack entry.
    let Some(removed) = removed else {
        return ToolReply::Failed(format!("{} has no {short}", entity_id(entity)));
    };
    edits::record_component(
        world,
        entity,
        &full_path,
        format!("remove {short}"),
        Some(removed),
        None,
    );
    ToolReply::Text(format!("removed {short} from {}", entity_id(entity)))
}

pub(super) fn set_component_field(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("set_component_field needs an entity id".into());
    };
    let (Some(component), Some(field)) = (
        args.get("component").and_then(Value::as_str),
        args.get("field").and_then(Value::as_str),
    ) else {
        return ToolReply::Failed("set_component_field needs a component and a field".into());
    };
    let Some(raw) = args.get("value") else {
        return ToolReply::Failed("set_component_field needs a value".into());
    };
    let Some(value) = property_value(raw) else {
        return ToolReply::Failed(
            "a value must be a number, a boolean, a string, or 3 or 4 numbers".into(),
        );
    };

    if edits::set_field(world, entity, component, field, value.clone()) {
        return ToolReply::Text(format!(
            "set {component}.{field} on {}",
            entity_id(entity)
        ));
    }

    // A float field rejects an Int and vice versa, and JSON cannot tell 1 from
    // 1.0. Rather than making the caller guess which one the field wants, try the
    // other reading of a whole number before reporting failure.
    if let PropertyValue::Float(f) = value {
        if f.fract() == 0.0
            && edits::set_field(world, entity, component, field, PropertyValue::Int(f as i64))
        {
            return ToolReply::Text(format!(
                "set {component}.{field} on {}",
                entity_id(entity)
            ));
        }
    }

    ToolReply::Failed(format!(
        "could not set {component}.{field}: no such component or field on {}, or the value does not fit it",
        entity_id(entity)
    ))
}

/// Attach a script file to an entity, or change the preview flag of one already
/// attached.
///
/// A typed tool because `add_component` cannot reach this one: `ScriptComponent`
/// has no `#[reflect(Default)]`, and its payload is a `Vec` of entries that the
/// reflected field setter has no vocabulary for. Without this an agent could
/// write a script and watch it compile, and then had no way to run it.
///
/// The path is stored project-relative with forward slashes, which is what the
/// inspector and the hierarchy's drag-and-drop both store. An absolute path
/// inside the project is accepted and made relative, because an agent that has
/// just written the file knows it by its absolute path.
pub(super) fn attach_script(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("attach_script needs an entity id".into());
    };
    let Some(raw) = args.get("path").and_then(Value::as_str) else {
        return ToolReply::Failed("attach_script needs a path".into());
    };
    if world.get_entity(entity).is_err() {
        return ToolReply::Failed(format!("{} is not a live entity", entity_id(entity)));
    }
    let rel = match project_relative(world, raw) {
        Ok(rel) => rel,
        Err(why) => return ToolReply::Failed(why),
    };
    let preview = args.get("preview").and_then(Value::as_bool);

    let mut scripts = world
        .get::<ScriptComponent>(entity)
        .cloned()
        .unwrap_or_default();
    let existing = scripts
        .scripts
        .iter()
        .position(|e| e.script_path.as_deref() == Some(rel.as_path()));
    let index = match existing {
        Some(index) => index,
        None => {
            scripts.add_file_script(rel.clone());
            scripts.scripts.len() - 1
        }
    };
    if let Some(preview) = preview {
        scripts.scripts[index].preview = preview;
    }
    let running = scripts.scripts[index].preview;
    edits::set_scripts(world, entity, Some(scripts));

    let shown = rel.to_string_lossy();
    let verb = if existing.is_some() { "already had" } else { "attached" };
    ToolReply::Text(if running {
        format!("{verb} {shown} on {}; previewing, so it runs now in edit mode", entity_id(entity))
    } else {
        format!(
            "{verb} {shown} on {}; it runs in play or simulate, or with preview: true",
            entity_id(entity)
        )
    })
}

pub(super) fn detach_script(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("detach_script needs an entity id".into());
    };
    let Some(raw) = args.get("path").and_then(Value::as_str) else {
        return ToolReply::Failed("detach_script needs a path".into());
    };
    let rel = match project_relative(world, raw) {
        Ok(rel) => rel,
        Err(why) => return ToolReply::Failed(why),
    };
    let Some(mut scripts) = world.get::<ScriptComponent>(entity).cloned() else {
        return ToolReply::Failed(format!("{} has no scripts", entity_id(entity)));
    };
    let before = scripts.scripts.len();
    scripts
        .scripts
        .retain(|e| e.script_path.as_deref() != Some(rel.as_path()));
    if scripts.scripts.len() == before {
        return ToolReply::Failed(format!(
            "{} is not attached to {}",
            rel.display(),
            entity_id(entity)
        ));
    }
    // The last script going takes the component with it, as the inspector does:
    // an empty one still serialises into the scene and still matches every query
    // that drives execution.
    let after = (!scripts.scripts.is_empty()).then_some(scripts);
    edits::set_scripts(world, entity, after);
    ToolReply::Text(format!("detached {} from {}", rel.display(), entity_id(entity)))
}

/// A script path as the Scripts component stores it: relative to the project,
/// forward slashes, and pointing at a file that exists.
///
/// Checked for existence here because a wrong path attaches without complaint
/// and then simply never runs, which is the most confusing way for this to fail.
fn project_relative(world: &World, raw: &str) -> Result<PathBuf, String> {
    let Some(project) = world.get_resource::<CurrentProject>().map(|p| p.path.clone()) else {
        return Err("no project is open".into());
    };
    let given = PathBuf::from(raw);
    let rel = if given.is_absolute() {
        given
            .strip_prefix(&project)
            .map(Path::to_path_buf)
            .map_err(|_| format!("{raw} is outside the open project at {}", project.display()))?
    } else {
        given
    };
    if !project.join(&rel).is_file() {
        return Err(format!("no file at {} in the project", rel.display()));
    }
    Ok(PathBuf::from(rel.to_string_lossy().replace('\\', "/")))
}
