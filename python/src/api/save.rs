//! Saved games (`save_*`), through `renzora::save`.
//!
//! Called straight into the contract crate rather than sent as commands, for
//! the Lua plugin's reason: a save is read back in the same hook that asks for
//! it (`ok, why = save_load("slot1")` then `save_get("level")`), and a command
//! would not land until the hook had returned.
//!
//! The slot functions return `(ok, message)`, a tuple standing in for Lua's
//! two return values: `(True, None)` on success, `(False, "why")` on failure.

use renzora::component::ScriptValue;
use renzora::save;
use rustpython_vm::builtins::PyDict;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::{AsObject, Py, PyObjectRef, PyResult, VirtualMachine};

use super::add;
use super::args::Args;
use crate::convert;

/// Python value to saved value. A whole number stays an integer, so `coins`
/// comes back as `42` rather than `42.0`, unless it does not fit one.
fn to_saved(value: &PyObjectRef, vm: &VirtualMachine) -> PyResult<ScriptValue> {
    match convert::to_script_value(value, vm) {
        Some(v) => Ok(v),
        None => Err(vm.new_type_error(format!(
            "save_set: a {} cannot be saved; use a number, bool, str, vec2, vec3 or color",
            value.class().name()
        ))),
    }
}

fn outcome(vm: &VirtualMachine, result: Result<(), save::SaveError>) -> PyObjectRef {
    let (ok, why) = match result {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };
    vm.ctx
        .new_tuple(vec![convert::boolean(vm, ok), convert::opt_text(vm, why.as_deref())])
        .into()
}

pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    add(vm, m, "save_set", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "save_set");
        let key = a.str(0, "key")?;
        let Some(value) = args.args.get(1) else {
            return Err(vm.new_type_error("save_set() missing required argument 'value'".to_owned()));
        };
        save::set(key, to_saved(value, vm)?);
        Ok(())
    });
    // `save_get(key, default=None)`.
    add(vm, m, "save_get", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let key = Args::new(&args, vm, "save_get").str(0, "key")?;
        match save::get(&key) {
            Some(v) => convert::from_script_value(vm, &v),
            None => Ok(args.args.get(1).cloned().unwrap_or_else(|| vm.ctx.none())),
        }
    });
    add(vm, m, "save_has", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let key = Args::new(&args, vm, "save_has").str(0, "key")?;
        Ok(save::get(&key).is_some())
    });
    add(vm, m, "save_remove", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let key = Args::new(&args, vm, "save_remove").str(0, "key")?;
        Ok(save::remove(&key))
    });
    add(vm, m, "save_clear", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<()> {
        save::clear();
        Ok(())
    });
    add(vm, m, "save_keys", |_args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        Ok(convert::text_list(vm, &save::keys()))
    });
    add(vm, m, "save_write", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let slot = Args::new(&args, vm, "save_write").str(0, "slot")?;
        Ok(outcome(vm, save::write(&slot)))
    });
    add(vm, m, "save_load", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let slot = Args::new(&args, vm, "save_load").str(0, "slot")?;
        Ok(outcome(vm, save::load(&slot)))
    });
    add(vm, m, "save_delete", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let slot = Args::new(&args, vm, "save_delete").str(0, "slot")?;
        Ok(outcome(vm, save::delete(&slot)))
    });
    add(vm, m, "save_exists", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let slot = Args::new(&args, vm, "save_exists").str(0, "slot")?;
        Ok(save::exists(&slot))
    });
    // A list of `{"name": "slot1", "saved_at": 1730000000}`, newest first.
    add(vm, m, "save_slots", |_args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let mut list = Vec::new();
        for slot in save::list() {
            let entry = vm.ctx.new_dict();
            entry.set_item("name", convert::text(vm, &slot.name), vm)?;
            entry.set_item("saved_at", convert::int(vm, slot.saved_at as i64), vm)?;
            list.push(entry.into());
        }
        Ok(vm.ctx.new_list(list).into())
    });
}
