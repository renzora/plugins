//! JSON, as C# sends it, to and from the engine's value types.
//!
//! Every value that is not on a hot path crosses the boundary as JSON: the
//! arguments of a `Call`, its result, an event's arguments, the props. The C#
//! side writes it with `System.Text.Json`'s writer and reads it with its
//! reader; nothing on either side reflects over types to do it.
//!
//! The engine has four value enums, each narrower than the last:
//! [`ScriptValue`] (script variables; has `Vec2` and `Color`), [`PropValue`]
//! (reflection reads and writes; no `Vec2`), [`ActionValue`] (action, event and
//! RPC arguments; no `Vec2`, no `Color`) and JSON. As in the other language
//! plugins, ONE classifier decides what a value means as a vector ([`shape`])
//! and every converter shares it, so the same value never means two things at
//! two boundaries.
//!
//! # Numbers keep their C# type
//!
//! Unlike JavaScript, C# knows whether it meant `5` or `5.0`, and the encoder
//! says so: a `float` or `double` is always written with a fraction (`5.0`), an
//! `int` or `long` never. So an integer here is a C# integer, and becomes an
//! `Int`, and anything with a fraction a `Float`.

use std::collections::HashMap;

use bevy::math::{Vec2, Vec3, Vec4};
use renzora::serde_json::{self, Map, Number, Value};
use renzora::{PropertyValue as PropValue, ScriptActionValue as ActionValue, ScriptValue};

/// A JSON value recognised as engine data.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shape {
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Color([f32; 4]),
}

pub fn number(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// An integer, as C# wrote it: no fraction in the text.
pub fn integer(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_u64().map(|u| u as i64))
}

/// A whole number, also from a float that happens to be one (`2.0`), for the
/// parameters that want a count or an index.
pub fn whole(v: &Value) -> Option<i64> {
    integer(v).or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.trunc() as i64))
}

/// A value as a log line or a string parameter would show it: a string as
/// itself, anything else as its JSON.
pub fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

/// Recognise a value as a vector or a colour.
///
/// The precedence of the other plugins: named fields beat positional ones, and
/// the widest match wins. Named fields are an object with numeric `x` and `y`
/// (and `z`), or `r`, `g` and `b` (and `a`), which is how the C# encoder writes
/// `Vector2`, `Vector3` and `Color`. Positional ones are a 2, 3 or 4 number
/// array. A value matching nothing is `None`, so a caller can refuse a write
/// instead of storing a zero.
pub fn shape(v: &Value) -> Option<Shape> {
    match v {
        Value::Array(items) => {
            let nums: Option<Vec<f32>> = items.iter().map(|v| number(v).map(|n| n as f32)).collect();
            match nums?.as_slice() {
                [x, y] => Some(Shape::Vec2([*x, *y])),
                [x, y, z] => Some(Shape::Vec3([*x, *y, *z])),
                [r, g, b, a] => Some(Shape::Color([*r, *g, *b, *a])),
                _ => None,
            }
        }
        Value::Object(map) => {
            let field = |k: &str| map.get(k).and_then(number).map(|n| n as f32);
            if let (Some(x), Some(y)) = (field("x"), field("y")) {
                return Some(match field("z") {
                    Some(z) => Shape::Vec3([x, y, z]),
                    None => Shape::Vec2([x, y]),
                });
            }
            if let (Some(r), Some(g), Some(b)) = (field("r"), field("g"), field("b")) {
                return Some(Shape::Color([r, g, b, field("a").unwrap_or(1.0)]));
            }
            None
        }
        _ => None,
    }
}

/// The name a script author would give a JSON value's type, for errors.
pub fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => match shape(v) {
            Some(Shape::Vec2(_)) => "Vector2",
            Some(Shape::Vec3(_)) => "Vector3",
            Some(Shape::Color(_)) => "Color",
            None => "object",
        },
    }
}

fn num(f: f32) -> Value {
    Number::from_f64(f as f64).map(Value::Number).unwrap_or(Value::Null)
}

pub fn vec2(x: f32, y: f32) -> Value {
    let mut m = Map::new();
    m.insert("x".into(), num(x));
    m.insert("y".into(), num(y));
    Value::Object(m)
}

pub fn vec3([x, y, z]: [f32; 3]) -> Value {
    let mut m = Map::new();
    m.insert("x".into(), num(x));
    m.insert("y".into(), num(y));
    m.insert("z".into(), num(z));
    Value::Object(m)
}

pub fn color([r, g, b, a]: [f32; 4]) -> Value {
    let mut m = Map::new();
    m.insert("r".into(), num(r));
    m.insert("g".into(), num(g));
    m.insert("b".into(), num(b));
    m.insert("a".into(), num(a));
    Value::Object(m)
}

pub fn text_list<S: AsRef<str>>(items: &[S]) -> Value {
    Value::Array(items.iter().map(|s| Value::String(s.as_ref().to_string())).collect())
}

pub fn opt_text(s: Option<&str>) -> Value {
    s.map(|s| Value::String(s.to_string())).unwrap_or(Value::Null)
}

// =============================================================================
// C# -> engine
// =============================================================================

/// A reflection write's value. `None` for anything unrecognised, so `Set`
/// throws instead of writing a zero into whatever field the path named.
///
/// `PropertyValue` has no `Vec2`, so a two-component value is promoted to a
/// `Vec3` with `z = 0`, the compromise every language plugin makes.
pub fn to_property_value(v: &Value) -> Option<PropValue> {
    match v {
        Value::Bool(b) => Some(PropValue::Bool(*b)),
        Value::Number(_) => Some(match integer(v) {
            Some(i) => PropValue::Int(i),
            None => PropValue::Float(number(v)? as f32),
        }),
        Value::String(s) => Some(PropValue::String(s.clone())),
        _ => Some(match shape(v)? {
            Shape::Vec2([x, y]) => PropValue::Vec3([x, y, 0.0]),
            Shape::Vec3(v) => PropValue::Vec3(v),
            Shape::Color(v) => PropValue::Color(v),
        }),
    }
}

/// An action, event or RPC argument.
///
/// The narrowest target. A vector degrades as in the other plugins (`Vector2`
/// gains `z = 0`, a colour loses its alpha). Any other object or array is sent
/// as its JSON text, which the receiving script can parse, and `null` as
/// `"nil"`, the string Lua sends for its `nil`, so a value means the same thing
/// whichever language sent it.
pub fn to_action_value(v: &Value) -> ActionValue {
    match v {
        Value::Bool(b) => ActionValue::Bool(*b),
        Value::Number(_) => match integer(v) {
            Some(i) => ActionValue::Int(i),
            None => ActionValue::Float(number(v).unwrap_or(0.0) as f32),
        },
        Value::String(s) => ActionValue::String(s.clone()),
        Value::Null => ActionValue::String("nil".into()),
        _ => match shape(v) {
            Some(Shape::Vec2([x, y])) => ActionValue::Vec3([x, y, 0.0]),
            Some(Shape::Vec3(v)) => ActionValue::Vec3(v),
            Some(Shape::Color([r, g, b, _])) => ActionValue::Vec3([r, g, b]),
            None => ActionValue::String(v.to_string()),
        },
    }
}

/// An `Action(name, new { key = value })` argument object, as name and value
/// pairs. Missing or `null` is no arguments; anything but an object is a
/// mistake worth hearing about.
pub fn action_args(func: &str, v: Option<&Value>) -> Result<Vec<(String, ActionValue)>, String> {
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) if shape(v.unwrap_or(&Value::Null)).is_none() => {
            Ok(map.iter().map(|(k, v)| (k.clone(), to_action_value(v))).collect())
        }
        Some(other) => Err(format!(
            "{func}(): arguments must be an object (a dictionary or an anonymous object), not {}",
            type_name(other)
        )),
    }
}

/// A saved-game value. An integer stays an `Int`, so `coins` comes back as
/// `42` in every language.
pub fn to_saved_value(v: &Value) -> Option<ScriptValue> {
    match v {
        Value::Bool(b) => Some(ScriptValue::Bool(*b)),
        Value::Number(_) => match integer(v).and_then(|i| i32::try_from(i).ok()) {
            Some(i) => Some(ScriptValue::Int(i)),
            None => Some(ScriptValue::Float(number(v)? as f32)),
        },
        Value::String(s) => Some(ScriptValue::String(s.clone())),
        _ => Some(match shape(v)? {
            Shape::Vec2(v) => ScriptValue::Vec2(Vec2::from_array(v)),
            Shape::Vec3(v) => ScriptValue::Vec3(Vec3::from_array(v)),
            Shape::Color(v) => ScriptValue::Color(Vec4::from_array(v)),
        }),
    }
}

/// A prop value C# handed back, typed by what the script declared (`kind` is
/// the load report's type name), so an `int` prop stays an `Int` and an
/// `Entity` prop stays an `Entity`.
pub fn to_prop_value(v: &Value, kind: &str) -> Option<ScriptValue> {
    Some(match kind {
        "float" => ScriptValue::Float(number(v)? as f32),
        "int" => ScriptValue::Int(whole(v)?.clamp(i32::MIN as i64, i32::MAX as i64) as i32),
        "bool" => ScriptValue::Bool(v.as_bool()?),
        "string" => ScriptValue::String(v.as_str()?.to_string()),
        "entity" => ScriptValue::Entity(v.as_str()?.to_string()),
        "vec2" => match shape(v)? {
            Shape::Vec2(xy) => ScriptValue::Vec2(Vec2::from_array(xy)),
            Shape::Vec3([x, y, _]) => ScriptValue::Vec2(Vec2::new(x, y)),
            Shape::Color(_) => return None,
        },
        "vec3" => match shape(v)? {
            Shape::Vec3(xyz) => ScriptValue::Vec3(Vec3::from_array(xyz)),
            Shape::Vec2([x, y]) => ScriptValue::Vec3(Vec3::new(x, y, 0.0)),
            Shape::Color(_) => return None,
        },
        "color" => match shape(v)? {
            Shape::Color(c) => ScriptValue::Color(Vec4::from_array(c)),
            Shape::Vec3([r, g, b]) => ScriptValue::Color(Vec4::new(r, g, b, 1.0)),
            Shape::Vec2(_) => return None,
        },
        _ => return None,
    })
}

// =============================================================================
// engine -> C#
// =============================================================================

pub fn from_script_value(value: &ScriptValue) -> Value {
    match value {
        ScriptValue::Float(v) => num(*v),
        ScriptValue::Int(v) => Value::from(*v),
        ScriptValue::Bool(v) => Value::Bool(*v),
        ScriptValue::String(v) | ScriptValue::Entity(v) => Value::String(v.clone()),
        ScriptValue::Vec2(v) => vec2(v.x, v.y),
        ScriptValue::Vec3(v) => vec3(v.to_array()),
        ScriptValue::Color(v) => color(v.to_array()),
    }
}

pub fn from_property_value(value: PropValue) -> Value {
    match value {
        PropValue::Float(v) => num(v),
        PropValue::Int(v) => Value::from(v),
        PropValue::Bool(v) => Value::Bool(v),
        PropValue::String(v) => Value::String(v),
        PropValue::Vec3(v) => vec3(v),
        PropValue::Color(v) => color(v),
    }
}

pub fn from_action_value(value: &ActionValue) -> Value {
    match value {
        ActionValue::Float(v) => num(*v),
        ActionValue::Int(v) => Value::from(*v),
        ActionValue::Bool(v) => Value::Bool(*v),
        ActionValue::String(v) => Value::String(v.clone()),
        ActionValue::Vec3(v) => vec3(*v),
    }
}

/// The `args` an `OnEvent` / `OnRpc` / `OnUi` hook receives, as JSON text.
pub fn args_json(args: &HashMap<String, ActionValue>) -> String {
    let map: Map<String, Value> = args.iter().map(|(k, v)| (k.clone(), from_action_value(v))).collect();
    Value::Object(map).to_string()
}

/// The inspector's values for one hook, as a JSON object.
pub fn props_json(vars: &[(String, ScriptValue)]) -> String {
    let map: Map<String, Value> = vars.iter().map(|(k, v)| (k.clone(), from_script_value(v))).collect();
    Value::Object(map).to_string()
}

pub fn parse(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok()
}
