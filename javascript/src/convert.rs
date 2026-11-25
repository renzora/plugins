//! JavaScript values to and from the engine's value types.
//!
//! The engine has four value enums, each narrower than the last:
//! [`ScriptValue`] (script variables; has `Vec2` and `Color`), [`PropValue`]
//! (reflection reads and writes; no `Vec2`), [`ActionValue`] (action, event and
//! RPC arguments; no `Vec2`, no `Color`) and JSON. JavaScript has no vector type,
//! so the prelude defines three classes, `Vec2`, `Vec3` and `Color`: objects with
//! `x` / `y` / `z` (or `r` / `g` / `b` / `a`) fields, because `{x, y, z}` is what
//! JavaScript code already writes for a point, and what `JSON.parse` hands back.
//!
//! As in the Lua and Python plugins, ONE classifier decides what a value means
//! as a vector ([`shape`]) and every converter shares it, so the same value
//! never means two things at two boundaries.
//!
//! # Numbers
//!
//! JavaScript has one number type where the engine has two, so an integral
//! number is sent as an `Int` and any other as a `Float`, which is what Lua and
//! Python send for `5` and `5.5`. The engine's reflection writes an `Int` into a
//! float field without complaint, so `set("Light.intensity", 2)` works. Props
//! are the exception, and are typed by what the inspector already holds (see
//! [`crate::context`]).

use std::collections::HashMap;

use bevy::math::{Vec2, Vec3, Vec4};
use boa_engine::object::builtins::JsArray;
use boa_engine::property::PropertyKey;
use boa_engine::{Context, JsNativeError, JsObject, JsResult, JsString, JsValue};
use renzora::{PropertyValue as PropValue, ScriptActionValue as ActionValue, ScriptValue};

/// The prelude's classes and helpers, which conversions in Rust need without
/// a trip through JavaScript. Kept in the `Context`'s own data slot, set once
/// when the runtime is built.
pub struct Prelude {
    pub vec2: JsObject,
    pub vec3: JsObject,
    pub color: JsObject,
    pub canvas: JsObject,
    /// `_inspect(value)`: how `console.log` and the console show a value.
    pub inspect: JsObject,
}

/// A JavaScript value recognised as engine data.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shape {
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Color([f32; 4]),
}

/// The value as a number, if it is one.
pub fn number(v: &JsValue) -> Option<f64> {
    v.as_number()
}

/// The value as a whole number, truncated toward zero as `Math.trunc` would.
/// `NaN` and the infinities are not whole numbers.
pub fn integer(v: &JsValue) -> Option<i64> {
    let n = v.as_number()?;
    n.is_finite().then(|| n.trunc() as i64)
}

/// A number that is exactly an integer and survives the trip through `i64`.
fn integral(n: f64) -> Option<i64> {
    // 2^53: past it every double is an integer, and most are not the integer
    // the script meant.
    (n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_992.0).then_some(n as i64)
}

/// A string as a Rust string. Lone surrogates (which JavaScript strings may
/// hold and Rust strings may not) become U+FFFD.
pub fn string(v: &JsValue) -> Option<String> {
    v.as_string().map(|s| s.to_std_string_lossy())
}

pub fn text(s: &str) -> JsValue {
    JsValue::from(JsString::from(s))
}

pub fn opt_text(s: Option<&str>) -> JsValue {
    s.map(text).unwrap_or_else(JsValue::null)
}

/// What `console.log` would print for `v`, for messages and for string
/// parameters given a non-string. Never fails: a value whose formatting throws
/// reads as Boa's own rendering of it.
pub fn display(v: &JsValue, ctx: &mut Context) -> String {
    if let Some(s) = string(v) {
        return s;
    }
    let inspect = ctx.get_data::<Prelude>().map(|p| p.inspect.clone());
    if let Some(inspect) = inspect {
        if let Ok(out) = inspect.call(&JsValue::undefined(), std::slice::from_ref(v), ctx) {
            if let Some(s) = string(&out) {
                return s;
            }
        }
    }
    v.display().to_string()
}

/// The items of an array, copied out. Only a real array: a string is iterable
/// too, and treating `"abc"` as three values is never what a script meant.
pub fn items(v: &JsValue, ctx: &mut Context) -> Option<Vec<JsValue>> {
    let obj = v.as_object()?;
    if !obj.is_array() {
        return None;
    }
    let array = JsArray::from_object(obj).ok()?;
    let len = array.length(ctx).ok()?;
    (0..len).map(|i| array.get(i, ctx).ok()).collect()
}

/// A plain object's own string-keyed entries, in property order.
pub fn entries(obj: &JsObject, ctx: &mut Context) -> JsResult<Vec<(String, JsValue)>> {
    let mut out = Vec::new();
    for key in obj.own_property_keys(ctx)? {
        let name = match &key {
            PropertyKey::String(s) => s.to_std_string_lossy(),
            PropertyKey::Index(i) => i.get().to_string(),
            PropertyKey::Symbol(_) => continue,
        };
        let value = obj.get(key, ctx)?;
        out.push((name, value));
    }
    Ok(out)
}

/// Recognise a value as a vector or a colour.
///
/// The same precedence as the Lua plugin's `classify_table`: named fields beat
/// positional ones, and the widest match wins. Named fields are any object with
/// numeric `x` and `y` (and `z`), or `r`, `g` and `b` (and `a`), which covers
/// the prelude's `Vec2` / `Vec3` / `Color` and a bare `{x: 1, y: 2}` alike.
/// Positional ones are a 2, 3 or 4 number array. A value matching nothing is
/// `None`, so a caller can refuse a write instead of storing a zero.
pub fn shape(v: &JsValue, ctx: &mut Context) -> Option<Shape> {
    let obj = v.as_object()?;
    if obj.is_array() {
        let list = items(v, ctx)?;
        let nums: Option<Vec<f32>> = list.iter().map(|v| number(v).map(|n| n as f32)).collect();
        return match nums?.as_slice() {
            [x, y] => Some(Shape::Vec2([*x, *y])),
            [x, y, z] => Some(Shape::Vec3([*x, *y, *z])),
            [r, g, b, a] => Some(Shape::Color([*r, *g, *b, *a])),
            _ => None,
        };
    }
    if obj.is_callable() {
        return None;
    }
    let mut field = |k: &'static str| -> Option<f32> {
        obj.get(JsString::from(k), ctx)
            .ok()
            .and_then(|v| number(&v))
            .map(|n| n as f32)
    };
    if let (Some(x), Some(y)) = (field("x"), field("y")) {
        return Some(match field("z") {
            Some(z) => Shape::Vec3([x, y, z]),
            None => Shape::Vec2([x, y]),
        });
    }
    if let (Some(r), Some(g), Some(b)) = (field("r"), field("g"), field("b")) {
        let a = field("a").unwrap_or(1.0);
        return Some(Shape::Color([r, g, b, a]));
    }
    None
}

/// An instance of one of the prelude's classes with the given fields.
///
/// Built here rather than by calling the class, which would be a JavaScript
/// call per vector: `get("Transform.translation")` in a loop over entities
/// should cost a property write per field, not a function call.
fn instance(ctx: &mut Context, proto: Option<JsObject>, fields: &[(&'static str, f32)]) -> JsResult<JsValue> {
    let obj = JsObject::with_object_proto(ctx.intrinsics());
    if let Some(proto) = proto {
        obj.set_prototype(Some(proto));
    }
    for (k, v) in fields {
        obj.create_data_property_or_throw(JsString::from(*k), *v as f64, ctx)?;
    }
    Ok(obj.into())
}

pub fn vec2(ctx: &mut Context, x: f32, y: f32) -> JsResult<JsValue> {
    let proto = ctx.get_data::<Prelude>().map(|p| p.vec2.clone());
    instance(ctx, proto, &[("x", x), ("y", y)])
}

pub fn vec3(ctx: &mut Context, [x, y, z]: [f32; 3]) -> JsResult<JsValue> {
    let proto = ctx.get_data::<Prelude>().map(|p| p.vec3.clone());
    instance(ctx, proto, &[("x", x), ("y", y), ("z", z)])
}

pub fn color(ctx: &mut Context, [r, g, b, a]: [f32; 4]) -> JsResult<JsValue> {
    let proto = ctx.get_data::<Prelude>().map(|p| p.color.clone());
    instance(ctx, proto, &[("r", r), ("g", g), ("b", b), ("a", a)])
}

/// An array of strings, for `get_components`, `save_keys` and the like.
pub fn text_list<S: AsRef<str>>(ctx: &mut Context, items: &[S]) -> JsValue {
    JsArray::from_iter(items.iter().map(|s| text(s.as_ref())), ctx).into()
}

// =============================================================================
// JavaScript -> engine
// =============================================================================

/// A script variable, as the inspector stores it.
///
/// `null`, `undefined` (and anything else it cannot carry) give `None`, which
/// leaves the stored value alone rather than clearing it. A number becomes a
/// `Float` here; [`crate::context::read_back`] narrows it to an `Int` when the
/// prop already is one.
pub fn to_script_value(v: &JsValue, ctx: &mut Context) -> Option<ScriptValue> {
    if let Some(b) = v.as_boolean() {
        return Some(ScriptValue::Bool(b));
    }
    if let Some(n) = number(v) {
        return Some(ScriptValue::Float(n as f32));
    }
    if let Some(s) = string(v) {
        return Some(ScriptValue::String(s));
    }
    Some(match shape(v, ctx)? {
        Shape::Vec2(v) => ScriptValue::Vec2(Vec2::from_array(v)),
        Shape::Vec3(v) => ScriptValue::Vec3(Vec3::from_array(v)),
        Shape::Color(v) => ScriptValue::Color(Vec4::from_array(v)),
    })
}

/// A value for a saved game. Like [`to_script_value`], except that an integral
/// number is stored as an `Int`, so `coins` comes back as `42` in every
/// language rather than as `42.0` in the ones that can tell.
pub fn to_saved_value(v: &JsValue, ctx: &mut Context) -> Option<ScriptValue> {
    if let Some(n) = number(v) {
        if let Some(i) = integral(n).and_then(|i| i32::try_from(i).ok()) {
            return Some(ScriptValue::Int(i));
        }
    }
    to_script_value(v, ctx)
}

/// A reflection write's value. `None` for anything unrecognised, so `set`
/// throws instead of writing a zero into whatever field the path named.
///
/// `PropertyValue` has no `Vec2`, so a two-component value is promoted to a
/// `Vec3` with `z = 0`, the same compromise the Lua plugin makes.
pub fn to_property_value(v: &JsValue, ctx: &mut Context) -> Option<PropValue> {
    if let Some(b) = v.as_boolean() {
        return Some(PropValue::Bool(b));
    }
    if let Some(n) = number(v) {
        return Some(match integral(n) {
            Some(i) => PropValue::Int(i),
            None => PropValue::Float(n as f32),
        });
    }
    if let Some(s) = string(v) {
        return Some(PropValue::String(s));
    }
    Some(match shape(v, ctx)? {
        Shape::Vec2([x, y]) => PropValue::Vec3([x, y, 0.0]),
        Shape::Vec3(v) => PropValue::Vec3(v),
        Shape::Color(v) => PropValue::Color(v),
    })
}

/// An action, event or RPC argument.
///
/// The narrowest target. A vector degrades as in Lua (`Vec2` gains `z = 0`, a
/// colour loses its alpha). Where Lua sends the fixed marker `"<table>"` for a
/// table it cannot read, any other object or array here is sent as its JSON
/// text: the receiving script can `json_parse` it, which a marker never
/// allowed. `null` and `undefined` are sent as `"nil"`, the string Lua sends
/// for its `nil`, so a value means the same thing whichever language sent it.
pub fn to_action_value(v: &JsValue, ctx: &mut Context) -> ActionValue {
    if let Some(b) = v.as_boolean() {
        return ActionValue::Bool(b);
    }
    if let Some(n) = number(v) {
        return match integral(n) {
            Some(i) => ActionValue::Int(i),
            None => ActionValue::Float(n as f32),
        };
    }
    if let Some(s) = string(v) {
        return ActionValue::String(s);
    }
    if v.is_null_or_undefined() {
        return ActionValue::String("nil".into());
    }
    match shape(v, ctx) {
        Some(Shape::Vec2([x, y])) => ActionValue::Vec3([x, y, 0.0]),
        Some(Shape::Vec3(v)) => ActionValue::Vec3(v),
        Some(Shape::Color([r, g, b, _])) => ActionValue::Vec3([r, g, b]),
        None => match to_json(v, ctx, 0) {
            Ok(json) => ActionValue::String(json.to_string()),
            Err(_) => ActionValue::String(display(v, ctx)),
        },
    }
}

/// An `action(name, {key: value})` argument object, as name and value pairs.
/// Missing, `null` or `undefined` is no arguments; anything but an object is a
/// mistake worth hearing about.
pub fn action_args(func: &str, v: Option<&JsValue>, ctx: &mut Context) -> JsResult<Vec<(String, ActionValue)>> {
    let Some(v) = v.filter(|v| !v.is_null_or_undefined()) else {
        return Ok(Vec::new());
    };
    let Some(obj) = v.as_object().filter(|o| !o.is_array() && !o.is_callable()) else {
        return Err(JsNativeError::typ()
            .with_message(format!("{func}(): arguments must be an object, not {}", v.type_of()))
            .into());
    };
    let mut out = Vec::new();
    for (k, v) in entries(&obj, ctx)? {
        let value = to_action_value(&v, ctx);
        out.push((k, value));
    }
    Ok(out)
}

// =============================================================================
// engine -> JavaScript
// =============================================================================

pub fn from_script_value(ctx: &mut Context, value: &ScriptValue) -> JsResult<JsValue> {
    Ok(match value {
        ScriptValue::Float(v) => JsValue::from(*v as f64),
        ScriptValue::Int(v) => JsValue::from(*v),
        ScriptValue::Bool(v) => JsValue::from(*v),
        ScriptValue::String(v) | ScriptValue::Entity(v) => text(v),
        ScriptValue::Vec2(v) => vec2(ctx, v.x, v.y)?,
        ScriptValue::Vec3(v) => vec3(ctx, v.to_array())?,
        ScriptValue::Color(v) => color(ctx, v.to_array())?,
    })
}

pub fn from_property_value(ctx: &mut Context, value: PropValue) -> JsResult<JsValue> {
    Ok(match value {
        PropValue::Float(v) => JsValue::from(v as f64),
        PropValue::Int(v) => JsValue::from(v as f64),
        PropValue::Bool(v) => JsValue::from(v),
        PropValue::String(v) => text(&v),
        PropValue::Vec3(v) => vec3(ctx, v)?,
        PropValue::Color(v) => color(ctx, v)?,
    })
}

pub fn from_action_value(ctx: &mut Context, value: &ActionValue) -> JsResult<JsValue> {
    Ok(match value {
        ActionValue::Float(v) => JsValue::from(*v as f64),
        ActionValue::Int(v) => JsValue::from(*v as f64),
        ActionValue::Bool(v) => JsValue::from(*v),
        ActionValue::String(v) => text(v),
        ActionValue::Vec3(v) => vec3(ctx, *v)?,
    })
}

/// The `args` object an `on_rpc` / `on_ui` / `on_event` hook receives.
pub fn args_object(ctx: &mut Context, args: &HashMap<String, ActionValue>) -> JsResult<JsValue> {
    let obj = JsObject::with_object_proto(ctx.intrinsics());
    for (k, v) in args {
        let value = from_action_value(ctx, v)?;
        obj.create_data_property_or_throw(JsString::from(k.as_str()), value, ctx)?;
    }
    Ok(obj.into())
}

// =============================================================================
// JSON
// =============================================================================

/// Decode JSON into objects, arrays, strings, numbers, booleans and `null`.
pub fn from_json(ctx: &mut Context, value: &serde_json::Value) -> JsResult<JsValue> {
    use serde_json::Value as J;
    Ok(match value {
        J::Null => JsValue::null(),
        J::Bool(b) => JsValue::from(*b),
        J::Number(n) => JsValue::from(n.as_f64().unwrap_or(0.0)),
        J::String(s) => text(s),
        J::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for e in arr {
                out.push(from_json(ctx, e)?);
            }
            JsArray::from_iter(out, ctx).into()
        }
        J::Object(map) => {
            let obj = JsObject::with_object_proto(ctx.intrinsics());
            for (k, v) in map {
                let value = from_json(ctx, v)?;
                obj.create_data_property_or_throw(JsString::from(k.as_str()), value, ctx)?;
            }
            obj.into()
        }
    })
}

/// How deep `to_json` follows nesting before deciding it has met a cycle. An
/// array that contains itself would otherwise recurse until the stack ran out.
const JSON_MAX_DEPTH: usize = 128;

/// Encode a value as JSON, as `JSON.stringify` would for data: a `Vec3` is the
/// object `{"x":..,"y":..,"z":..}`, `undefined` and functions in an object are
/// skipped and become `null` in an array, and `NaN` and the infinities become
/// `null`. A whole number is written without a fraction (`3`, not `3.0`).
pub fn to_json(v: &JsValue, ctx: &mut Context, depth: usize) -> JsResult<serde_json::Value> {
    use serde_json::Value as J;
    if depth > JSON_MAX_DEPTH {
        return Err(JsNativeError::range()
            .with_message("json_encode: value is nested too deeply (a cycle?)")
            .into());
    }
    if v.is_null_or_undefined() {
        return Ok(J::Null);
    }
    if let Some(b) = v.as_boolean() {
        return Ok(J::Bool(b));
    }
    if let Some(n) = number(v) {
        return Ok(match integral(n) {
            Some(i) => J::from(i),
            None => serde_json::Number::from_f64(n).map(J::Number).unwrap_or(J::Null),
        });
    }
    if let Some(s) = string(v) {
        return Ok(J::String(s));
    }
    if let Some(list) = items(v, ctx) {
        let mut out = Vec::with_capacity(list.len());
        for item in &list {
            out.push(if item.is_callable() {
                J::Null
            } else {
                to_json(item, ctx, depth + 1)?
            });
        }
        return Ok(J::Array(out));
    }
    if let Some(obj) = v.as_object().filter(|o| !o.is_callable()) {
        let mut map = serde_json::Map::new();
        for (k, item) in entries(&obj, ctx)? {
            if item.is_undefined() || item.is_callable() {
                continue;
            }
            map.insert(k, to_json(&item, ctx, depth + 1)?);
        }
        return Ok(J::Object(map));
    }
    Err(JsNativeError::typ()
        .with_message(format!("json_encode: a {} cannot be encoded", v.type_of()))
        .into())
}

/// A property key for a name only known at run time, kept here so every module
/// spells the conversion one way.
pub fn key(name: &str) -> JsString {
    JsString::from(name)
}
