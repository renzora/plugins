//! Reading and writing any reflected component field by path, whole components,
//! and the asset and scene load trackers.
//!
//! `set` / `get` are the generic escape hatch: anything the engine reflects is
//! reachable without this plugin learning about it. The reads are synchronous,
//! answered mid-hook by the engine's `get_handler`, which is thread-local and
//! set up on the thread running the hook: the thread C# calls back on.

use renzora::serde_json::{Map, Value};
use renzora::{get_handler as host, ScriptCommand};

use super::{push, Args, Out};
use crate::convert;

/// "ComponentType.field.subfield" into ("ComponentType", "field.subfield").
pub fn split_path(path: &str) -> Option<(String, String)> {
    let (component, field) = path.split_once('.')?;
    if component.is_empty() || field.is_empty() {
        return None;
    }
    Some((component.to_string(), field.to_string()))
}

fn path_of(a: &Args, i: usize) -> Result<(String, String), String> {
    let path = a.str(i, "path")?;
    split_path(&path).ok_or_else(|| format!("{}(): invalid path '{path}'; use 'Component.field'", a.func))
}

fn write(a: &Args, entity_name: Option<String>, path_at: usize) -> Out {
    let (component, field) = path_of(a, path_at)?;
    let raw = a.raw(path_at + 1);
    let Some(value) = convert::to_property_value(&raw) else {
        return Err(format!(
            "{}(): cannot write a {} to '{component}.{field}'; use a number, bool, string, Vector3 or Color",
            a.func,
            convert::type_name(&raw)
        ));
    };
    push(ScriptCommand::SetComponentField {
        entity_id: None,
        entity_name,
        component_type: component,
        field_path: field,
        value,
    })
}

/// One field, as JSON; `null` when the entity has no such field.
pub fn read(entity: Option<&str>, component: &str, field: &str) -> Value {
    host::call_get(entity, component, field)
        .map(convert::from_property_value)
        .unwrap_or(Value::Null)
}

fn component_object(entity: Option<&str>, component: &str) -> Value {
    match host::call_get_component(entity, component) {
        Some(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k, convert::from_property_value(v)))
                .collect::<Map<String, Value>>(),
        ),
        None => Value::Null,
    }
}

pub fn call(name: &str, a: &Args) -> Option<Out> {
    Some(match name {
        // set("ComponentType.field.subfield", value), on this entity.
        "set" => write(a, None, 0),
        // set_on("EntityName", "ComponentType.field", value)
        "set_on" => a.str(0, "entity").and_then(|e| write(a, Some(e), 1)),
        "get" => path_of(a, 0).map(|(c, f)| read(None, &c, &f)),
        "get_on" => (|| {
            let entity = a.str(0, "entity")?;
            let (c, f) = path_of(a, 1)?;
            Ok(read(Some(&entity), &c, &f))
        })(),
        "get_component" => a.str(0, "component").map(|c| component_object(None, &c)),
        "get_component_on" => (|| {
            let entity = a.str(0, "entity")?;
            let c = a.str(1, "component")?;
            Ok(component_object(Some(&entity), &c))
        })(),
        "get_components" => Ok(convert::text_list(&host::call_get_components(None))),
        "get_components_on" => a
            .str(0, "entity")
            .map(|e| convert::text_list(&host::call_get_components(Some(&e)))),
        "has_component" => a
            .str(0, "component")
            .map(|c| Value::Bool(host::call_get_component(None, &c).is_some())),
        "has_component_on" => (|| {
            let entity = a.str(0, "entity")?;
            let c = a.str(1, "component")?;
            Ok(Value::Bool(host::call_get_component(Some(&entity), &c).is_some()))
        })(),

        // asset_progress(): an object, or `null` when nothing is loading. Keys:
        // state ("idle" / "loading" / "done"), total_files, loaded_files,
        // total_bytes, loaded_bytes, fraction (0..1), current_path, elapsed_secs.
        "asset_progress" => Ok(match host::call_asset_progress() {
            None => Value::Null,
            Some(s) => {
                let mut m = Map::new();
                m.insert("state".into(), Value::String(s.state.into()));
                m.insert("total_files".into(), Value::from(s.total_files));
                m.insert("loaded_files".into(), Value::from(s.loaded_files));
                m.insert("total_bytes".into(), Value::from(s.total_bytes));
                m.insert("loaded_bytes".into(), Value::from(s.loaded_bytes));
                m.insert("fraction".into(), Value::from(s.fraction as f64));
                m.insert("elapsed_secs".into(), Value::from(s.elapsed_secs as f64));
                m.insert("current_path".into(), convert::opt_text(s.current_path.as_deref()));
                Value::Object(m)
            }
        }),
        // scene_load_state(): phase ("idle" / "loading" / "ready" / "failed"),
        // current_path, progress; `null` before any load.
        "scene_load_state" => Ok(match host::call_scene_load() {
            None => Value::Null,
            Some(s) => {
                let mut m = Map::new();
                m.insert("phase".into(), Value::String(s.phase.into()));
                m.insert("progress".into(), Value::from(s.progress as f64));
                m.insert("current_path".into(), convert::opt_text(s.current_path.as_deref()));
                Value::Object(m)
            }
        }),
        "is_loading" => Ok(Value::Bool(host::call_asset_progress().is_some_and(|s| s.state == "loading"))),
        "is_loaded" => Ok(Value::Bool(host::call_asset_progress().is_some_and(|s| s.state == "done"))),
        _ => return None,
    })
}
