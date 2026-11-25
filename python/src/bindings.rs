//! Functions other crates and plugins declare through `ScriptExtension`.
//!
//! `apply_force`, `nav_set_destination`, `tr`, the MySQL plugin's
//! `mysql_query(sql, callback)`: none of them is written for Python. Each is a
//! [`Binding`] (a name, a kind, typed parameters) that every language backend
//! builds a function from, and this is where Python builds them.
//!
//! # One dispatcher, generated wrappers
//!
//! The obvious shape, one native function per binding capturing its
//! description, is the one shape RustPython makes expensive: it leaks every
//! native callable it is given, and capturing closures are not zero-sized. So
//! there is a single native `_call_binding(name, args)`, which looks the
//! binding up in a thread-local table, and each binding is a two-line Python
//! `def` that forwards to it, compiled in one go from generated source. The
//! `def`s are ordinary functions, so they show up in `dir()` and tracebacks
//! under their own names.
//!
//! The wrappers live in `builtins`, shared by every script on the thread, so a
//! changed binding list is applied by rewriting them in place: no script has
//! to be rebuilt to see a function a late-registering plugin added.

use std::cell::RefCell;
use std::collections::HashMap;

use renzora::script_extension::{substitute, Binding, BindingKind, ParamKind};
use renzora::{get_handler as host, ScriptActionValue as ActionValue, ScriptCommand};
use rustpython_vm::builtins::PyDict;
use rustpython_vm::compiler::Mode;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::scope::Scope;
use rustpython_vm::{Py, PyObjectRef, PyResult, VirtualMachine};

use crate::buffers::push_command;
use crate::convert;

thread_local! {
    /// The bindings the wrappers on this thread forward to, by name.
    static TABLE: RefCell<HashMap<String, Binding>> = RefCell::new(HashMap::new());
    /// The names installed into `builtins` last time, so a binding that went
    /// away can be taken out again.
    static INSTALLED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Python keywords, which cannot be function names. A binding named one of
/// these is reachable as `renzora.<name>` via `getattr` only.
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
    "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
    "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise",
    "return", "try", "while", "with", "yield",
];

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        && !KEYWORDS.contains(&name)
}

/// Make `bindings` callable from every script on this thread.
///
/// `module` is the `renzora` module's dict, which receives every binding;
/// `builtins` receives each one too, except a name that is already a Python
/// builtin the plugin did not put there (`print`, `len`, ...): shadowing one
/// of those would break the standard library for every script at once, so
/// such a binding is reachable as `renzora.<name>` only, with a warning.
/// `api_names` are the plugin's own functions, which a binding may override,
/// as a binding registered after `register_api` overrides it in Lua.
pub fn install(
    vm: &VirtualMachine,
    module: &Py<PyDict>,
    api_names: &[String],
    bindings: &[Binding],
) -> PyResult<()> {
    let builtins = vm.builtins.dict();

    // Take out what the previous list installed, so a removed binding does
    // not linger as a function that forwards to nothing.
    for name in INSTALLED.with(|i| std::mem::take(&mut *i.borrow_mut())) {
        let _ = builtins.del_item(name.as_str(), vm);
        if !api_names.contains(&name) {
            let _ = module.del_item(name.as_str(), vm);
        }
    }

    let mut source = String::new();
    let mut names = Vec::new();
    for b in bindings {
        if !is_identifier(&b.name) {
            bevy::log::warn!(
                "[Python] binding `{}` is not a valid Python name and was skipped",
                b.name
            );
            continue;
        }
        source.push_str(&format!(
            "def {name}(*args):\n    return _call_binding({name:?}, args)\n",
            name = b.name
        ));
        names.push(b.name.clone());
    }
    TABLE.with(|t| {
        *t.borrow_mut() = bindings
            .iter()
            .map(|b| (b.name.clone(), b.clone()))
            .collect()
    });
    if names.is_empty() {
        return Ok(());
    }

    let globals = vm.ctx.new_dict();
    globals.set_item("_call_binding", module.get_item("_call_binding", vm)?, vm)?;
    let code = vm
        .compile(&source, Mode::Exec, "<renzora bindings>".to_string())
        .map_err(|e| vm.new_syntax_error(&e, Some(&source)))?;
    vm.run_code_obj(code, Scope::with_builtins(None, globals.clone(), vm))?;

    let mut installed = Vec::new();
    for name in names {
        let Some(func) = globals.get_item_opt(name.as_str(), vm)? else {
            continue;
        };
        module.set_item(name.as_str(), func.clone(), vm)?;
        let taken = builtins.get_item_opt(name.as_str(), vm)?.is_some();
        if taken && !api_names.contains(&name) {
            bevy::log::warn!(
                "[Python] binding `{name}` would hide Python's own `{name}`; \
                 call it as `renzora.{name}(...)` instead"
            );
            continue;
        }
        builtins.set_item(name.as_str(), func, vm)?;
        installed.push(name);
    }
    INSTALLED.with(|i| *i.borrow_mut() = installed);
    Ok(())
}

/// The one native every generated wrapper calls: `_call_binding(name, args)`.
pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    crate::api::add(vm, m, "_call_binding", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let name = args
            .args
            .first()
            .and_then(|v| convert::string(v))
            .unwrap_or_default();
        let call_args = args
            .args
            .get(1)
            .and_then(|v| convert::items(v))
            .unwrap_or_default();
        let Some(binding) = TABLE.with(|t| t.borrow().get(&name).cloned()) else {
            return Err(vm.new_name_error(
                format!("script function `{name}` is no longer registered"),
                vm.ctx.new_str(name.as_str()),
            ));
        };
        call(vm, &binding, &call_args)
    });
}

fn call(vm: &VirtualMachine, b: &Binding, args: &[PyObjectRef]) -> PyResult {
    match &b.kind {
        BindingKind::Action { action } => {
            let mut packed = Vec::with_capacity(b.params.len());
            let mut i = 0;
            for p in &b.params {
                let (value, used) = arg_value(vm, args, i, p.kind);
                packed.push((p.name.clone(), value));
                i += used;
            }
            push_command(ScriptCommand::Action {
                name: action.clone(),
                target_entity: None,
                args: packed,
            });
            Ok(vm.ctx.none())
        }
        BindingKind::Read { component, field } => {
            // Placeholders are positional over the SCRIPT arguments, not the
            // packed ones, so a `Vec3` parameter does not shift the numbering.
            let subs: Vec<String> = (0..b.params.len().max(args.len()))
                .map(|i| args.get(i).map(|v| convert::display(v, vm)).unwrap_or_default())
                .collect();
            let component = substitute(component, &subs);
            let field = substitute(field, &subs);
            match host::call_get(None, &component, &field) {
                Some(v) => convert::from_property_value(vm, v),
                None => Ok(vm.ctx.none()),
            }
        }
        BindingKind::Translate => {
            let key = args.first().map(|v| convert::display(v, vm)).unwrap_or_default();
            Ok(convert::text(vm, &renzora::lang::t(&key)))
        }
    }
}

/// One declared parameter from the script's arguments, and how many arguments
/// it consumed.
///
/// A missing or wrong-typed argument becomes the type's zero rather than an
/// error, matching the Lua plugin (and the hand-written bindings before it):
/// scripts depend on `apply_force(1, 2)` meaning a `z` of zero.
///
/// Two conveniences Lua cannot offer: a `Vec3` parameter takes one `vec3` (or
/// any 3-item tuple) as well as three numbers, and a `Str` parameter given a
/// `dict` or `list` receives its JSON text, so `mysql_query_with(sql, [3],
/// "scores")` works without a `json_encode`.
fn arg_value(vm: &VirtualMachine, args: &[PyObjectRef], i: usize, kind: ParamKind) -> (ActionValue, usize) {
    let v = args.get(i).filter(|v| !vm.is_none(v));
    let f = |v: Option<&PyObjectRef>| v.and_then(|v| convert::number(v, vm)).unwrap_or(0.0) as f32;
    match kind {
        ParamKind::Float => (ActionValue::Float(f(v)), 1),
        ParamKind::Int => (
            ActionValue::Int(v.and_then(|v| convert::integer(v, vm)).unwrap_or(0)),
            1,
        ),
        ParamKind::Bool => (
            ActionValue::Bool(v.is_some_and(|v| v.clone().is_true(vm).unwrap_or(false))),
            1,
        ),
        ParamKind::Str => (
            ActionValue::String(match v {
                None => String::new(),
                Some(v) => match convert::string(v) {
                    Some(s) => s,
                    None if v.downcast_ref::<PyDict>().is_some()
                        || v.downcast_ref::<rustpython_vm::builtins::PyList>().is_some() =>
                    {
                        convert::to_json(v, vm, 0)
                            .map(|j| j.to_string())
                            .unwrap_or_else(|_| convert::display(v, vm))
                    }
                    None => convert::display(v, vm),
                },
            }),
            1,
        ),
        ParamKind::Vec3 => match v.and_then(|v| {
            convert::number(v, vm)
                .is_none()
                .then(|| convert::shape(v, vm))
                .flatten()
        }) {
            Some(convert::Shape::Vec3(xyz)) => (ActionValue::Vec3(xyz), 1),
            Some(convert::Shape::Vec2([x, y])) => (ActionValue::Vec3([x, y, 0.0]), 1),
            _ => (
                ActionValue::Vec3([f(args.get(i)), f(args.get(i + 1)), f(args.get(i + 2))]),
                // Consumes three script arguments; see `ParamKind::arity`.
                kind.arity(),
            ),
        },
    }
}
