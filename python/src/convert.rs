//! Python values to and from the engine's value types.
//!
//! The engine has four value enums, each narrower than the last:
//! [`ScriptValue`] (script variables; has `Vec2` and `Color`), [`PropValue`]
//! (reflection reads and writes; no `Vec2`), [`ActionValue`] (action, event and
//! RPC arguments; no `Vec2`, no `Color`) and JSON. Python has no vector type at
//! all, so the plugin's prelude defines three: `vec2`, `vec3` and `color`, each
//! a `tuple` subclass with named fields. A tuple because that is what Python
//! code already writes for a point (`(1, 2, 3)`), and unpacks (`x, y, z = v`);
//! a subclass so it can also be read as `v.x`.
//!
//! As in the Lua plugin, ONE classifier decides what a Python value means as a
//! vector ([`shape`]) and every converter shares it, so the same value never
//! means two things at two boundaries.

use std::collections::HashMap;

use bevy::math::{Vec2, Vec3, Vec4};
use renzora::{
    PropertyValue as PropValue, ScriptActionValue as ActionValue, ScriptValue,
};
use rustpython_vm::builtins::{PyDict, PyDictRef, PyFloat, PyInt, PyList, PyStr, PyTuple};
use rustpython_vm::function::FuncArgs;
use rustpython_vm::{AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine};

/// A Python value recognised as engine data.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shape {
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Color([f32; 4]),
}

/// The value as a number, if it is one. `bool` counts, as it does in Python.
pub fn number(obj: &PyObject, vm: &VirtualMachine) -> Option<f64> {
    if let Some(f) = obj.downcast_ref::<PyFloat>() {
        return Some(f.to_f64());
    }
    if is_bool(obj, vm) {
        return Some(if obj.is(&vm.ctx.true_value) { 1.0 } else { 0.0 });
    }
    if let Some(i) = obj.downcast_ref::<PyInt>() {
        return i
            .try_to_primitive::<i64>(vm)
            .map(|v| v as f64)
            .ok()
            .or_else(|| rustpython_vm::builtins::try_bigint_to_f64(i.as_bigint(), vm).ok());
    }
    None
}

/// The value as a whole number. A float is truncated, as `int()` would.
pub fn integer(obj: &PyObject, vm: &VirtualMachine) -> Option<i64> {
    if is_bool(obj, vm) {
        return Some(obj.is(&vm.ctx.true_value) as i64);
    }
    if let Some(i) = obj.downcast_ref::<PyInt>() {
        return i.try_to_primitive::<i64>(vm).ok();
    }
    obj.downcast_ref::<PyFloat>().map(|f| f.to_f64() as i64)
}

pub fn is_bool(obj: &PyObject, vm: &VirtualMachine) -> bool {
    obj.class().is(vm.ctx.types.bool_type)
}

/// A `str` as a Rust string.
pub fn string(obj: &PyObject) -> Option<String> {
    obj.downcast_ref::<PyStr>().map(|s| s.as_str().to_owned())
}

/// What `str(obj)` says, for messages and for string parameters given a
/// non-string. Never fails: an object whose `__str__` raises reads as its type.
pub fn display(obj: &PyObject, vm: &VirtualMachine) -> String {
    if let Some(s) = string(obj) {
        return s;
    }
    match obj.str(vm) {
        Ok(s) => s.as_str().to_owned(),
        Err(_) => format!("<{}>", obj.class().name()),
    }
}

/// The items of a `list` or `tuple` (a `vec3` included), copied out.
///
/// Only those two: a `str` is a sequence too, and treating `"abc"` as three
/// values is never what a script meant.
pub fn items(obj: &PyObject) -> Option<Vec<PyObjectRef>> {
    if let Some(t) = obj.downcast_ref::<PyTuple>() {
        return Some(t.as_slice().to_vec());
    }
    obj.downcast_ref::<PyList>().map(|l| l.borrow_vec().to_vec())
}

/// Recognise a value as a vector or a colour.
///
/// The same precedence as the Lua plugin's `classify_table`: named fields beat
/// positional ones, and the widest match wins. Named fields are a `dict`
/// (`{"x": 1, "y": 2}`, which is what `json_parse` hands back); positional ones
/// are a 2, 3 or 4 item tuple or list, which covers `vec2`/`vec3`/`color` since
/// those are tuples. A value matching nothing is `None`, so a caller can refuse
/// a write instead of storing a zero.
pub fn shape(obj: &PyObject, vm: &VirtualMachine) -> Option<Shape> {
    if let Some(d) = obj.downcast_ref::<PyDict>() {
        let field = |k: &str| {
            d.get_item_opt(k, vm)
                .ok()
                .flatten()
                .and_then(|v| number(&v, vm))
        };
        if let (Some(x), Some(y)) = (field("x"), field("y")) {
            return Some(match field("z") {
                Some(z) => Shape::Vec3([x as f32, y as f32, z as f32]),
                None => Shape::Vec2([x as f32, y as f32]),
            });
        }
        if let (Some(r), Some(g), Some(b)) = (field("r"), field("g"), field("b")) {
            let a = field("a").unwrap_or(1.0);
            return Some(Shape::Color([r as f32, g as f32, b as f32, a as f32]));
        }
        return None;
    }
    let list = items(obj)?;
    let nums: Option<Vec<f32>> = list.iter().map(|v| number(v, vm).map(|n| n as f32)).collect();
    match nums?.as_slice() {
        [x, y] => Some(Shape::Vec2([*x, *y])),
        [x, y, z] => Some(Shape::Vec3([*x, *y, *z])),
        [r, g, b, a] => Some(Shape::Color([*r, *g, *b, *a])),
        _ => None,
    }
}

/// Build a `vec2`/`vec3`/`color` from the prelude.
///
/// Looked up in `builtins` on every call rather than cached: it is one dict
/// lookup, and a cached class object would have to be kept per interpreter
/// (one per thread) for no measurable gain.
pub fn make(vm: &VirtualMachine, class: &'static str, values: &[f32]) -> PyResult {
    let ctor = vm.builtins.get_attr(class, vm)?;
    let args: Vec<PyObjectRef> = values
        .iter()
        .map(|v| vm.ctx.new_float(*v as f64).into())
        .collect();
    ctor.call(FuncArgs::from(args), vm)
}

pub fn vec2(vm: &VirtualMachine, x: f32, y: f32) -> PyResult {
    make(vm, "vec2", &[x, y])
}

pub fn vec3(vm: &VirtualMachine, v: [f32; 3]) -> PyResult {
    make(vm, "vec3", &v)
}

pub fn color(vm: &VirtualMachine, v: [f32; 4]) -> PyResult {
    make(vm, "color", &v)
}

pub fn float(vm: &VirtualMachine, v: f64) -> PyObjectRef {
    vm.ctx.new_float(v).into()
}

pub fn int(vm: &VirtualMachine, v: i64) -> PyObjectRef {
    vm.ctx.new_int(v).into()
}

pub fn boolean(vm: &VirtualMachine, v: bool) -> PyObjectRef {
    vm.ctx.new_bool(v).into()
}

pub fn text(vm: &VirtualMachine, v: &str) -> PyObjectRef {
    vm.ctx.new_str(v).into()
}

pub fn opt_text(vm: &VirtualMachine, v: Option<&str>) -> PyObjectRef {
    match v {
        Some(s) => text(vm, s),
        None => vm.ctx.none(),
    }
}

/// A list of strings, for `get_components`, `save_keys` and the like.
pub fn text_list<S: AsRef<str>>(vm: &VirtualMachine, items: &[S]) -> PyObjectRef {
    let items: Vec<PyObjectRef> = items.iter().map(|s| text(vm, s.as_ref())).collect();
    vm.ctx.new_list(items).into()
}

// =============================================================================
// Python -> engine
// =============================================================================

/// A script variable, as the inspector stores it.
///
/// `None` (and anything else it cannot carry) gives `None`, which leaves the
/// stored value alone rather than clearing it.
pub fn to_script_value(obj: &PyObject, vm: &VirtualMachine) -> Option<ScriptValue> {
    if is_bool(obj, vm) {
        return Some(ScriptValue::Bool(obj.is(&vm.ctx.true_value)));
    }
    if obj.downcast_ref::<PyInt>().is_some() {
        // The inspector stores an `i32`. A value past it keeps its magnitude
        // as a float rather than wrapping into a different number.
        let n = integer(obj, vm)?;
        return Some(match i32::try_from(n) {
            Ok(n) => ScriptValue::Int(n),
            Err(_) => ScriptValue::Float(n as f32),
        });
    }
    if let Some(f) = obj.downcast_ref::<PyFloat>() {
        return Some(ScriptValue::Float(f.to_f64() as f32));
    }
    if let Some(s) = string(obj) {
        return Some(ScriptValue::String(s));
    }
    Some(match shape(obj, vm)? {
        Shape::Vec2(v) => ScriptValue::Vec2(Vec2::from_array(v)),
        Shape::Vec3(v) => ScriptValue::Vec3(Vec3::from_array(v)),
        Shape::Color(v) => ScriptValue::Color(Vec4::from_array(v)),
    })
}

/// A reflection write's value. `None` for anything unrecognised, so `set`
/// raises instead of writing a zero into whatever field the path named.
///
/// `PropertyValue` has no `Vec2`, so a two-component value is promoted to a
/// `Vec3` with `z = 0`, the same compromise the Lua plugin makes.
pub fn to_property_value(obj: &PyObject, vm: &VirtualMachine) -> Option<PropValue> {
    if is_bool(obj, vm) {
        return Some(PropValue::Bool(obj.is(&vm.ctx.true_value)));
    }
    if obj.downcast_ref::<PyInt>().is_some() {
        return Some(PropValue::Int(integer(obj, vm)?));
    }
    if let Some(f) = obj.downcast_ref::<PyFloat>() {
        return Some(PropValue::Float(f.to_f64() as f32));
    }
    if let Some(s) = string(obj) {
        return Some(PropValue::String(s));
    }
    Some(match shape(obj, vm)? {
        Shape::Vec2([x, y]) => PropValue::Vec3([x, y, 0.0]),
        Shape::Vec3(v) => PropValue::Vec3(v),
        Shape::Color(v) => PropValue::Color(v),
    })
}

/// An action, event or RPC argument.
///
/// The narrowest target. A vector degrades as in Lua (`Vec2` gains `z = 0`, a
/// colour loses its alpha). Where Lua sends the fixed marker `"<table>"` for a
/// table it cannot read, a `dict` or `list` here is sent as its JSON text: the
/// receiving script can `json_parse` it, which a marker never allowed.
/// `None` is sent as `"nil"`, the string Lua sends for its `nil`, so a value
/// means the same thing whichever language sent it.
pub fn to_action_value(obj: &PyObject, vm: &VirtualMachine) -> ActionValue {
    if is_bool(obj, vm) {
        return ActionValue::Bool(obj.is(&vm.ctx.true_value));
    }
    if obj.downcast_ref::<PyInt>().is_some() {
        return match integer(obj, vm) {
            Some(n) => ActionValue::Int(n),
            None => ActionValue::Float(number(obj, vm).unwrap_or(0.0) as f32),
        };
    }
    if let Some(f) = obj.downcast_ref::<PyFloat>() {
        return ActionValue::Float(f.to_f64() as f32);
    }
    if let Some(s) = string(obj) {
        return ActionValue::String(s);
    }
    if vm.is_none(obj) {
        return ActionValue::String("nil".into());
    }
    match shape(obj, vm) {
        Some(Shape::Vec2([x, y])) => ActionValue::Vec3([x, y, 0.0]),
        Some(Shape::Vec3(v)) => ActionValue::Vec3(v),
        Some(Shape::Color([r, g, b, _])) => ActionValue::Vec3([r, g, b]),
        None => match to_json(obj, vm, 0) {
            Ok(json) => ActionValue::String(json.to_string()),
            Err(_) => ActionValue::String(display(obj, vm)),
        },
    }
}

/// An `action(name, {..}, key=value)` argument list: the dict first, then the
/// keyword arguments, which win on a clash because they were written last.
pub fn action_args(
    dict: Option<&PyObjectRef>,
    kwargs: &FuncArgs,
    vm: &VirtualMachine,
) -> PyResult<Vec<(String, ActionValue)>> {
    let mut out: Vec<(String, ActionValue)> = Vec::new();
    if let Some(obj) = dict {
        if !vm.is_none(obj) {
            let Some(d) = obj.downcast_ref::<PyDict>() else {
                return Err(vm.new_type_error(format!(
                    "arguments must be a dict, not {}",
                    obj.class().name()
                )));
            };
            for (k, v) in d {
                out.push((display(&k, vm), to_action_value(&v, vm)));
            }
        }
    }
    for (k, v) in kwargs.kwargs.iter() {
        out.retain(|(name, _)| name != k);
        out.push((k.clone(), to_action_value(v, vm)));
    }
    Ok(out)
}

// =============================================================================
// engine -> Python
// =============================================================================

pub fn from_script_value(vm: &VirtualMachine, value: &ScriptValue) -> PyResult {
    Ok(match value {
        ScriptValue::Float(v) => float(vm, *v as f64),
        ScriptValue::Int(v) => int(vm, *v as i64),
        ScriptValue::Bool(v) => boolean(vm, *v),
        ScriptValue::String(v) | ScriptValue::Entity(v) => text(vm, v),
        ScriptValue::Vec2(v) => vec2(vm, v.x, v.y)?,
        ScriptValue::Vec3(v) => vec3(vm, v.to_array())?,
        ScriptValue::Color(v) => color(vm, v.to_array())?,
    })
}

pub fn from_property_value(vm: &VirtualMachine, value: PropValue) -> PyResult {
    Ok(match value {
        PropValue::Float(v) => float(vm, v as f64),
        PropValue::Int(v) => int(vm, v),
        PropValue::Bool(v) => boolean(vm, v),
        PropValue::String(v) => text(vm, &v),
        PropValue::Vec3(v) => vec3(vm, v)?,
        PropValue::Color(v) => color(vm, v)?,
    })
}

pub fn from_action_value(vm: &VirtualMachine, value: &ActionValue) -> PyResult {
    Ok(match value {
        ActionValue::Float(v) => float(vm, *v as f64),
        ActionValue::Int(v) => int(vm, *v),
        ActionValue::Bool(v) => boolean(vm, *v),
        ActionValue::String(v) => text(vm, v),
        ActionValue::Vec3(v) => vec3(vm, *v)?,
    })
}

/// The `args` dict an `on_rpc` / `on_ui` / `on_event` hook receives.
pub fn args_dict(vm: &VirtualMachine, args: &HashMap<String, ActionValue>) -> PyResult<PyDictRef> {
    let dict = vm.ctx.new_dict();
    for (k, v) in args {
        dict.set_item(k.as_str(), from_action_value(vm, v)?, vm)?;
    }
    Ok(dict)
}

// =============================================================================
// JSON
// =============================================================================

/// Decode JSON into `dict` / `list` / `str` / `int` / `float` / `bool` / `None`.
pub fn from_json(vm: &VirtualMachine, value: &serde_json::Value) -> PyResult {
    use serde_json::Value as J;
    Ok(match value {
        J::Null => vm.ctx.none(),
        J::Bool(b) => boolean(vm, *b),
        J::Number(n) => match n.as_i64() {
            Some(i) => int(vm, i),
            None => float(vm, n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => text(vm, s),
        J::Array(arr) => {
            let items: PyResult<Vec<PyObjectRef>> = arr.iter().map(|e| from_json(vm, e)).collect();
            vm.ctx.new_list(items?).into()
        }
        J::Object(map) => {
            let dict = vm.ctx.new_dict();
            for (k, v) in map {
                dict.set_item(k.as_str(), from_json(vm, v)?, vm)?;
            }
            dict.into()
        }
    })
}

/// How deep `json_encode` follows nesting before deciding it has met a cycle.
/// A list that contains itself would otherwise recurse until the stack ran out.
const JSON_MAX_DEPTH: usize = 128;

/// Encode a value as JSON. Tuples (a `vec3` included) become arrays; a `dict`
/// key that is not a string is written as its `str()`, as Python's own
/// `json.dumps` does for numbers.
pub fn to_json(obj: &PyObject, vm: &VirtualMachine, depth: usize) -> PyResult<serde_json::Value> {
    use serde_json::Value as J;
    if depth > JSON_MAX_DEPTH {
        return Err(vm.new_value_error(
            "json_encode: value is nested too deeply (a cycle?)".to_owned(),
        ));
    }
    if vm.is_none(obj) {
        return Ok(J::Null);
    }
    if is_bool(obj, vm) {
        return Ok(J::Bool(obj.is(&vm.ctx.true_value)));
    }
    if obj.downcast_ref::<PyInt>().is_some() {
        if let Some(n) = integer(obj, vm) {
            return Ok(J::from(n));
        }
        return Ok(serde_json::Number::from_f64(number(obj, vm).unwrap_or(0.0))
            .map(J::Number)
            .unwrap_or(J::Null));
    }
    if let Some(f) = obj.downcast_ref::<PyFloat>() {
        // JSON has no NaN or infinity; `null` is what browsers write for them.
        return Ok(serde_json::Number::from_f64(f.to_f64())
            .map(J::Number)
            .unwrap_or(J::Null));
    }
    if let Some(s) = string(obj) {
        return Ok(J::String(s));
    }
    if let Some(list) = items(obj) {
        let out: PyResult<Vec<J>> = list.iter().map(|v| to_json(v, vm, depth + 1)).collect();
        return Ok(J::Array(out?));
    }
    if let Some(d) = obj.downcast_ref::<PyDict>() {
        let mut map = serde_json::Map::new();
        for (k, v) in d {
            map.insert(display(&k, vm), to_json(&v, vm, depth + 1)?);
        }
        return Ok(J::Object(map));
    }
    Err(vm.new_type_error(format!(
        "json_encode: a {} cannot be encoded; use None, bool, int, float, str, list, tuple or dict",
        obj.class().name()
    )))
}
