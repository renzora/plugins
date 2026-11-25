//! Reading and writing any reflected component field by path, whole components,
//! and the asset and scene load trackers.
//!
//! `set` / `get` are the generic escape hatch: anything the engine reflects is
//! reachable without this plugin learning about it.
//!
//! `set` is the one API name that collides with a Python builtin. It is
//! registered in the `renzora` module only, never in `builtins` (the standard
//! library calls `set()` constantly), and each script instead gets the
//! prelude's `set` shim in its own globals, which does both jobs: `set(path,
//! value)` writes a field, and `set()` / `set(items)` is still Python's set.

use renzora::{get_handler as host, ScriptCommand};
use rustpython_vm::builtins::PyDict;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::{Py, PyObjectRef, PyResult, VirtualMachine};

use super::add;
use super::args::Args;
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

fn path_of(a: &Args, i: usize) -> PyResult<(String, String)> {
    let path = a.str(i, "path")?;
    split_path(&path).ok_or_else(|| {
        a.vm()
            .new_value_error(format!("Invalid path '{path}'. Use 'Component.field'"))
    })
}

fn write(a: &Args, entity_name: Option<String>, path_at: usize) -> PyResult<()> {
    let (component, field) = path_of(a, path_at)?;
    let vm = a.vm();
    let raw = a.raw().args.get(path_at + 1).cloned().unwrap_or_else(|| vm.ctx.none());
    let value = convert::to_property_value(&raw, vm).ok_or_else(|| {
        vm.new_type_error(format!(
            "cannot write a {} to '{component}.{field}'; use a number, bool, str, vec3 or color",
            raw.class().name()
        ))
    })?;
    push_command(ScriptCommand::SetComponentField {
        entity_id: None,
        entity_name,
        component_type: component,
        field_path: field,
        value,
    });
    Ok(())
}

fn read(vm: &VirtualMachine, entity: Option<&str>, component: &str, field: &str) -> PyResult {
    match host::call_get(entity, component, field) {
        Some(v) => convert::from_property_value(vm, v),
        None => Ok(vm.ctx.none()),
    }
}

fn component_dict(vm: &VirtualMachine, entity: Option<&str>, component: &str) -> PyResult {
    let Some(fields) = host::call_get_component(entity, component) else {
        return Ok(vm.ctx.none());
    };
    let dict = vm.ctx.new_dict();
    for (key, value) in fields {
        dict.set_item(key.as_str(), convert::from_property_value(vm, value)?, vm)?;
    }
    Ok(dict.into())
}

pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    // set("ComponentType.field.subfield", value), on this entity.
    add(vm, m, "set", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        write(&Args::new(&args, vm, "set"), None, 0)
    });
    // set_on("EntityName", "ComponentType.field", value)
    add(vm, m, "set_on", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "set_on");
        let entity = a.str(0, "entity")?;
        write(&a, Some(entity), 1)
    });
    // get("Component.field"), from this entity. `None` when it has no such field.
    add(vm, m, "get", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let (component, field) = path_of(&Args::new(&args, vm, "get"), 0)?;
        read(vm, None, &component, &field)
    });
    add(vm, m, "get_on", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let a = Args::new(&args, vm, "get_on");
        let entity = a.str(0, "entity")?;
        let (component, field) = path_of(&a, 1)?;
        read(vm, Some(&entity), &component, &field)
    });

    // Whole components, as a dict of field to value; `None` when absent.
    add(vm, m, "get_component", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let component = Args::new(&args, vm, "get_component").str(0, "component")?;
        component_dict(vm, None, &component)
    });
    add(vm, m, "get_component_on", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let a = Args::new(&args, vm, "get_component_on");
        let entity = a.str(0, "entity")?;
        let component = a.str(1, "component")?;
        component_dict(vm, Some(&entity), &component)
    });
    // Every reflected component name on the entity, as a list.
    add(vm, m, "get_components", |_args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        Ok(convert::text_list(vm, &host::call_get_components(None)))
    });
    add(vm, m, "get_components_on", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let entity = Args::new(&args, vm, "get_components_on").str(0, "entity")?;
        Ok(convert::text_list(vm, &host::call_get_components(Some(&entity))))
    });
    add(vm, m, "has_component", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let component = Args::new(&args, vm, "has_component").str(0, "component")?;
        Ok(host::call_get_component(None, &component).is_some())
    });
    add(vm, m, "has_component_on", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let a = Args::new(&args, vm, "has_component_on");
        let entity = a.str(0, "entity")?;
        let component = a.str(1, "component")?;
        Ok(host::call_get_component(Some(&entity), &component).is_some())
    });

    progress(vm, m);
}

/// The runtime asset-load tracker and the scene loader, for loading screens.
fn progress(vm: &VirtualMachine, m: &Py<PyDict>) {
    // asset_progress(): a dict, or `None` when nothing is loading. Keys: state
    // ("idle" / "loading" / "done"), total_files, loaded_files, total_bytes,
    // loaded_bytes, fraction (0..1), current_path, elapsed_secs.
    add(vm, m, "asset_progress", |_args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let Some(s) = host::call_asset_progress() else {
            return Ok(vm.ctx.none());
        };
        let d = vm.ctx.new_dict();
        let set = |k: &str, v: PyObjectRef| d.set_item(k, v, vm);
        set("state", convert::text(vm, s.state))?;
        set("total_files", convert::int(vm, s.total_files as i64))?;
        set("loaded_files", convert::int(vm, s.loaded_files as i64))?;
        set("total_bytes", convert::int(vm, s.total_bytes as i64))?;
        set("loaded_bytes", convert::int(vm, s.loaded_bytes as i64))?;
        set("fraction", convert::float(vm, s.fraction as f64))?;
        set("elapsed_secs", convert::float(vm, s.elapsed_secs as f64))?;
        set("current_path", convert::opt_text(vm, s.current_path.as_deref()))?;
        Ok(d.into())
    });
    // scene_load_state(): which scene is loading and how far through spawning
    // it is, or `None` before any load. Keys: phase ("idle" / "loading" /
    // "ready" / "failed"), current_path, progress. A scene reaches "ready"
    // while its models may still be streaming; wait on asset_progress() too.
    add(vm, m, "scene_load_state", |_args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let Some(s) = host::call_scene_load() else {
            return Ok(vm.ctx.none());
        };
        let d = vm.ctx.new_dict();
        d.set_item("phase", convert::text(vm, s.phase), vm)?;
        d.set_item("progress", convert::float(vm, s.progress as f64), vm)?;
        d.set_item("current_path", convert::opt_text(vm, s.current_path.as_deref()), vm)?;
        Ok(d.into())
    });
    add(vm, m, "is_loading", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<bool> {
        Ok(host::call_asset_progress().is_some_and(|s| s.state == "loading"))
    });
    add(vm, m, "is_loaded", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<bool> {
        Ok(host::call_asset_progress().is_some_and(|s| s.state == "done"))
    });
}
