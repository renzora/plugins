//! Reading and writing any reflected component field by path, whole components,
//! and the asset and scene load trackers.
//!
//! `set` / `get` are the generic escape hatch: anything the engine reflects is
//! reachable without this plugin learning about it. Unlike in Python, `set`
//! collides with nothing in JavaScript (`Set` is the collection), so it is an
//! ordinary global here.

use boa_engine::{Context, JsNativeError, JsObject, JsResult, JsValue};
use renzora::{get_handler as host, ScriptCommand};

use super::add;
use super::args::{type_name, Args};
use crate::buffers::push_command;
use crate::convert;

/// "ComponentType.field.subfield" into ("ComponentType", "field.subfield").
fn split_path(path: &str) -> Option<(String, String)> {
    let (component, field) = path.split_once('.')?;
    if component.is_empty() || field.is_empty() {
        return None;
    }
    Some((component.to_string(), field.to_string()))
}

fn path_of(a: &Args, i: usize) -> JsResult<(String, String)> {
    let path = a.str(i, "path")?;
    split_path(&path).ok_or_else(|| {
        JsNativeError::error()
            .with_message(format!("Invalid path '{path}'. Use 'Component.field'"))
            .into()
    })
}

fn write(a: &Args, ctx: &mut Context, entity_name: Option<String>, path_at: usize) -> JsResult<JsValue> {
    let (component, field) = path_of(a, path_at)?;
    let raw = a.raw(path_at + 1);
    let Some(value) = convert::to_property_value(&raw, ctx) else {
        return Err(JsNativeError::typ()
            .with_message(format!(
                "cannot write a {} to '{component}.{field}'; use a number, boolean, string, Vec3 or Color",
                type_name(&raw)
            ))
            .into());
    };
    push_command(ScriptCommand::SetComponentField {
        entity_id: None,
        entity_name,
        component_type: component,
        field_path: field,
        value,
    });
    Ok(JsValue::undefined())
}

fn read(ctx: &mut Context, entity: Option<&str>, component: &str, field: &str) -> JsResult<JsValue> {
    match host::call_get(entity, component, field) {
        Some(v) => convert::from_property_value(ctx, v),
        None => Ok(JsValue::null()),
    }
}

fn component_object(ctx: &mut Context, entity: Option<&str>, component: &str) -> JsResult<JsValue> {
    let Some(fields) = host::call_get_component(entity, component) else {
        return Ok(JsValue::null());
    };
    let obj = JsObject::with_object_proto(ctx.intrinsics());
    for (key, value) in fields {
        let value = convert::from_property_value(ctx, value)?;
        obj.create_data_property_or_throw(convert::key(&key), value, ctx)?;
    }
    Ok(obj.into())
}

pub fn register(ctx: &mut Context, m: &JsObject) {
    // set("ComponentType.field.subfield", value), on this entity.
    add(ctx, m, "set", |_, args, ctx| write(&Args::new(args, "set"), ctx, None, 0));
    // set_on("EntityName", "ComponentType.field", value)
    add(ctx, m, "set_on", |_, args, ctx| {
        let a = Args::new(args, "set_on");
        let entity = a.str(0, "entity")?;
        write(&a, ctx, Some(entity), 1)
    });
    // get("Component.field"), from this entity. `null` when it has no such field.
    add(ctx, m, "get", |_, args, ctx| {
        let (component, field) = path_of(&Args::new(args, "get"), 0)?;
        read(ctx, None, &component, &field)
    });
    add(ctx, m, "get_on", |_, args, ctx| {
        let a = Args::new(args, "get_on");
        let entity = a.str(0, "entity")?;
        let (component, field) = path_of(&a, 1)?;
        read(ctx, Some(&entity), &component, &field)
    });

    // Whole components, as an object of field to value; `null` when absent.
    add(ctx, m, "get_component", |_, args, ctx| {
        let component = Args::new(args, "get_component").str(0, "component")?;
        component_object(ctx, None, &component)
    });
    add(ctx, m, "get_component_on", |_, args, ctx| {
        let a = Args::new(args, "get_component_on");
        let entity = a.str(0, "entity")?;
        let component = a.str(1, "component")?;
        component_object(ctx, Some(&entity), &component)
    });
    // Every reflected component name on the entity, as an array.
    add(ctx, m, "get_components", |_, _, ctx| {
        Ok(convert::text_list(ctx, &host::call_get_components(None)))
    });
    add(ctx, m, "get_components_on", |_, args, ctx| {
        let entity = Args::new(args, "get_components_on").str(0, "entity")?;
        Ok(convert::text_list(ctx, &host::call_get_components(Some(&entity))))
    });
    add(ctx, m, "has_component", |_, args, _| {
        let component = Args::new(args, "has_component").str(0, "component")?;
        Ok(JsValue::from(host::call_get_component(None, &component).is_some()))
    });
    add(ctx, m, "has_component_on", |_, args, _| {
        let a = Args::new(args, "has_component_on");
        let entity = a.str(0, "entity")?;
        let component = a.str(1, "component")?;
        Ok(JsValue::from(host::call_get_component(Some(&entity), &component).is_some()))
    });

    progress(ctx, m);
}

/// The runtime asset-load tracker and the scene loader, for loading screens.
fn progress(ctx: &mut Context, m: &JsObject) {
    // asset_progress(): an object, or `null` when nothing is loading. Keys:
    // state ("idle" / "loading" / "done"), total_files, loaded_files,
    // total_bytes, loaded_bytes, fraction (0..1), current_path, elapsed_secs.
    add(ctx, m, "asset_progress", |_, _, ctx| {
        let Some(s) = host::call_asset_progress() else {
            return Ok(JsValue::null());
        };
        let d = JsObject::with_object_proto(ctx.intrinsics());
        let fields: [(&str, JsValue); 8] = [
            ("state", convert::text(s.state)),
            ("total_files", JsValue::from(s.total_files as f64)),
            ("loaded_files", JsValue::from(s.loaded_files as f64)),
            ("total_bytes", JsValue::from(s.total_bytes as f64)),
            ("loaded_bytes", JsValue::from(s.loaded_bytes as f64)),
            ("fraction", JsValue::from(s.fraction as f64)),
            ("elapsed_secs", JsValue::from(s.elapsed_secs as f64)),
            ("current_path", convert::opt_text(s.current_path.as_deref())),
        ];
        for (k, v) in fields {
            d.create_data_property_or_throw(convert::key(k), v, ctx)?;
        }
        Ok(d.into())
    });
    // scene_load_state(): which scene is loading and how far through spawning
    // it is, or `null` before any load. Keys: phase ("idle" / "loading" /
    // "ready" / "failed"), current_path, progress. A scene reaches "ready"
    // while its models may still be streaming; wait on asset_progress() too.
    add(ctx, m, "scene_load_state", |_, _, ctx| {
        let Some(s) = host::call_scene_load() else {
            return Ok(JsValue::null());
        };
        let d = JsObject::with_object_proto(ctx.intrinsics());
        d.create_data_property_or_throw(convert::key("phase"), convert::text(s.phase), ctx)?;
        d.create_data_property_or_throw(convert::key("progress"), s.progress as f64, ctx)?;
        d.create_data_property_or_throw(
            convert::key("current_path"),
            convert::opt_text(s.current_path.as_deref()),
            ctx,
        )?;
        Ok(d.into())
    });
    add(ctx, m, "is_loading", |_, _, _| {
        Ok(JsValue::from(host::call_asset_progress().is_some_and(|s| s.state == "loading")))
    });
    add(ctx, m, "is_loaded", |_, _, _| {
        Ok(JsValue::from(host::call_asset_progress().is_some_and(|s| s.state == "done")))
    });
}
