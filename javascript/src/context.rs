//! The per-call variables a script reads (`delta`, `position_x`, ...), and its
//! props.
//!
//! # Context variables are accessors
//!
//! The Lua and Python plugins write some sixty variables into each script's
//! globals before every hook, whether the script reads them or not. Here each
//! one is instead a getter on the global object, defined once per interpreter,
//! that reads the running hook's context through [`crate::frame`]. A hook pays
//! for the variables it reads and nothing for the rest, and the names are
//! still plain identifiers: `translate(input_x * speed * delta, 0, 0)` reads
//! the same in all three languages.
//!
//! A getter has no setter, which makes "these are inputs" literal rather than a
//! convention: `position_x = 3` does nothing in sloppy code and throws a
//! `TypeError` in strict code (a class body, or under `"use strict"`). A script
//! that wants a variable of that name declares its own (`let delta = ...`),
//! which shadows the global as any declaration does.
//!
//! # Props are globals for the length of a hook
//!
//! The inspector's values are written onto the global object just before a
//! hook, read back just after, and then removed, so one entity's `speed` is
//! never what another script sees. A script reads a prop as a free name,
//! exactly as in Lua and Python.

use bevy::math::{EulerRot, Quat, Vec3};
use renzora::context::ScriptTransform;
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{Context, JsObject, JsResult, JsValue, NativeFunction};
use renzora::{ScriptContext, ScriptValue, ScriptVariableDefinition as VarDef, GAMEPAD_BUTTON_NAMES};

use crate::convert;
use crate::frame;

/// One context variable: its name and how to read it.
///
/// A plain `fn` per variable keeps the table one line per name; the getter
/// each one becomes captures only its index into this table, which is `Copy`
/// and therefore the kind of capture Boa can build without `unsafe`.
type Read = fn(&ScriptContext) -> Value;

/// What a variable reads as, before it becomes a `JsValue` (which an array
/// cannot be built without the `Context`).
enum Value {
    Num(f64),
    Bool(bool),
    Text(String),
    List(Vec<String>),
}

fn euler(c: &ScriptContext) -> (f32, f32, f32) {
    // `XYZ` so x/y/z below mean what their names say.
    c.transform.rotation.to_euler(EulerRot::XYZ)
}

fn first_pad(c: &ScriptContext, read: fn(&renzora::GamepadSnapshot) -> f32) -> Value {
    Value::Num(c.gamepads.first().map(read).unwrap_or(0.0) as f64)
}

/// Every context variable but the per-button gamepad ones, under the Lua
/// plugin's names.
const VARIABLES: &[(&str, Read)] = &[
    // Time
    ("delta", |c| Value::Num(c.time.delta as f64)),
    ("elapsed", |c| Value::Num(c.time.elapsed)),
    // Transform
    ("position_x", |c| Value::Num(c.transform.position.x as f64)),
    ("position_y", |c| Value::Num(c.transform.position.y as f64)),
    ("position_z", |c| Value::Num(c.transform.position.z as f64)),
    ("rotation_x", |c| Value::Num(euler(c).0 as f64)),
    ("rotation_y", |c| Value::Num(euler(c).1 as f64)),
    ("rotation_z", |c| Value::Num(euler(c).2 as f64)),
    ("scale_x", |c| Value::Num(c.transform.scale.x as f64)),
    ("scale_y", |c| Value::Num(c.transform.scale.y as f64)),
    ("scale_z", |c| Value::Num(c.transform.scale.z as f64)),
    // Input
    ("input_x", |c| Value::Num(c.input_movement.x as f64)),
    ("input_y", |c| Value::Num(c.input_movement.y as f64)),
    ("mouse_x", |c| Value::Num(c.mouse_position.x as f64)),
    ("mouse_y", |c| Value::Num(c.mouse_position.y as f64)),
    ("mouse_delta_x", |c| Value::Num(c.mouse_delta.x as f64)),
    ("mouse_delta_y", |c| Value::Num(c.mouse_delta.y as f64)),
    ("camera_yaw", |c| Value::Num(c.camera_yaw as f64)),
    ("mouse_left", |c| Value::Bool(c.mouse_buttons_pressed[0])),
    ("mouse_right", |c| Value::Bool(c.mouse_buttons_pressed[1])),
    ("mouse_middle", |c| Value::Bool(c.mouse_buttons_pressed[2])),
    ("mouse_left_just_pressed", |c| Value::Bool(c.mouse_buttons_just_pressed[0])),
    ("mouse_right_just_pressed", |c| Value::Bool(c.mouse_buttons_just_pressed[1])),
    ("mouse_scroll", |c| Value::Num(c.mouse_scroll as f64)),
    // Camera exposure (live scene EV-100) and the configured game resolution.
    ("camera_ev", |c| Value::Num(c.camera_ev as f64)),
    ("project_width", |c| Value::Num(c.project_width as f64)),
    ("project_height", |c| Value::Num(c.project_height as f64)),
    // The first connected gamepad, for scripts written before multi-pad.
    ("gamepad_left_x", |c| first_pad(c, |p| p.left_stick.x)),
    ("gamepad_left_y", |c| first_pad(c, |p| p.left_stick.y)),
    ("gamepad_right_x", |c| first_pad(c, |p| p.right_stick.x)),
    ("gamepad_right_y", |c| first_pad(c, |p| p.right_stick.y)),
    ("gamepad_left_trigger", |c| first_pad(c, |p| p.left_trigger)),
    ("gamepad_right_trigger", |c| first_pad(c, |p| p.right_trigger)),
    // Entity. The id is an `Entity::to_bits()`, exact as a JavaScript number
    // for every entity a scene will ever hold (generation below 2^21).
    ("self_entity_id", |c| Value::Num(c.self_entity_id as f64)),
    ("self_entity_name", |c| Value::Text(c.self_entity_name.clone())),
    ("is_colliding", |c| Value::Bool(!c.active_collisions.is_empty())),
    ("timers_finished", |c| Value::List(c.timers_just_finished.clone())),
    ("self_health", |c| Value::Num(c.self_health as f64)),
    ("self_max_health", |c| Value::Num(c.self_max_health as f64)),
    // Parent
    ("has_parent", |c| Value::Bool(c.has_parent)),
    ("parent_position_x", |c| Value::Num(c.parent_position.x as f64)),
    ("parent_position_y", |c| Value::Num(c.parent_position.y as f64)),
    ("parent_position_z", |c| Value::Num(c.parent_position.z as f64)),
];

/// Which variable a getter reads.
#[derive(Clone, Copy)]
enum Var {
    Table(usize),
    /// `gamepad_<name>`, the first pad's button at this index.
    Button(usize),
}

thread_local! {
    /// What the variables read outside a hook (a `props()` parse, the
    /// console): an empty context, so a script's top level can mention
    /// `delta` and get `0` rather than a `ReferenceError`.
    static IDLE: ScriptContext = ScriptContext::new(
        Default::default(),
        ScriptTransform {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
}

fn read(var: Var, ctx: &mut Context) -> JsValue {
    let get = |c: &ScriptContext| match var {
        Var::Table(i) => (VARIABLES[i].1)(c),
        Var::Button(i) => Value::Bool(c.gamepads.first().is_some_and(|p| p.buttons[i])),
    };
    let value = frame::with(get).unwrap_or_else(|| IDLE.with(get));
    match value {
        Value::Num(n) => JsValue::from(n),
        Value::Bool(b) => JsValue::from(b),
        Value::Text(s) => convert::text(&s),
        Value::List(l) => convert::text_list(ctx, &l),
    }
}

/// Every context variable's name, so a prop of the same name can be refused
/// rather than replacing the variable for every script on the thread.
pub fn names() -> Vec<String> {
    let mut out: Vec<String> = VARIABLES.iter().map(|(n, _)| n.to_string()).collect();
    out.extend(GAMEPAD_BUTTON_NAMES.iter().map(|n| format!("gamepad_{n}")));
    out
}

/// Define every context variable as a getter on `global`. Once per
/// interpreter.
pub fn install(ctx: &mut Context, global: &JsObject) -> JsResult<()> {
    let mut vars: Vec<(String, Var)> = VARIABLES
        .iter()
        .enumerate()
        .map(|(i, (n, _))| (n.to_string(), Var::Table(i)))
        .collect();
    vars.extend(
        GAMEPAD_BUTTON_NAMES
            .iter()
            .enumerate()
            .map(|(i, n)| (format!("gamepad_{n}"), Var::Button(i))),
    );
    for (name, var) in vars {
        let getter = FunctionObjectBuilder::new(
            ctx.realm(),
            NativeFunction::from_copy_closure(move |_, _, ctx| Ok(read(var, ctx))),
        )
        .name(convert::key(&format!("get {name}")))
        .build();
        global.define_property_or_throw(
            convert::key(&name),
            PropertyDescriptor::builder()
                .get(getter)
                .enumerable(false)
                .configurable(false),
            ctx,
        )?;
    }
    Ok(())
}

// =============================================================================
// Props
// =============================================================================

/// Write the inspector's values onto the global object for one hook.
///
/// A prop named like a context variable or an engine function is skipped: it
/// would replace that name for every script on the thread, and the inspector
/// cannot have meant that.
pub fn set_props(ctx: &mut Context, global: &JsObject, vars: &[(String, ScriptValue)], reserved: &[String]) -> JsResult<()> {
    for (name, value) in vars {
        if reserved.iter().any(|r| r == name) {
            continue;
        }
        let value = convert::from_script_value(ctx, value)?;
        global.set(convert::key(name), value, false, ctx)?;
    }
    Ok(())
}

/// Read the props back after a hook, then take them off the global object.
///
/// Only the names the inspector already knows: a script assigning a fresh
/// global is not promoted to a prop, which keeps the inspector's rows stable.
/// A number is narrowed to what the prop already holds, so an `Int` prop stays
/// an `Int` after `count += 1` (JavaScript cannot say which it meant).
pub fn read_back(
    ctx: &mut Context,
    global: &JsObject,
    vars: &[(String, ScriptValue)],
    reserved: &[String],
) -> Vec<(String, ScriptValue)> {
    let mut out = Vec::with_capacity(vars.len());
    for (name, before) in vars {
        if reserved.iter().any(|r| r == name) {
            continue;
        }
        let key = convert::key(name);
        let Ok(value) = global.get(key.clone(), ctx) else {
            continue;
        };
        let _ = global.delete_property_or_throw(key, ctx);
        let narrowed = match (before, convert::number(&value)) {
            (ScriptValue::Int(_), Some(n)) if n.is_finite() => Some(ScriptValue::Int(n.round() as i32)),
            _ => convert::to_script_value(&value, ctx),
        };
        if let Some(sv) = narrowed {
            out.push((name.clone(), sv));
        }
    }
    out
}

/// The props a script declares in `props()`.
///
/// `props()` returns an object of name to either a bare default (`{speed: 5}`)
/// or an object carrying `value` (or `default`) plus optional `hint`, `tab`
/// and `type` (`{speed: {value: 5, hint: "metres per second"}}`), the same two
/// forms the Lua plugin reads from a table.
///
/// A number is a `Float` prop, because JavaScript writes `5` and `5.0` the
/// same and a speed is far likelier than a count; `type: "int"` makes it an
/// `Int`. Props keep the order they were written in.
pub fn props(ctx: &mut Context, func: &JsObject) -> Vec<VarDef> {
    let mut out = Vec::new();
    let Ok(result) = func.call(&JsValue::undefined(), &[], ctx) else {
        return out;
    };
    let Some(table) = result.as_object() else {
        return out;
    };
    let Ok(entries) = convert::entries(&table, ctx) else {
        return out;
    };
    for (name, value) in entries {
        let display_name = to_display_name(&name);
        let mut hint = None;
        let mut tab = None;
        let mut want_int = false;
        let mut default = value.clone();
        if let Some(spec) = value.as_object().filter(|o| !o.is_array() && convert::shape(&value, ctx).is_none()) {
            let mut field = |k: &str| spec.get(convert::key(k), ctx).ok().filter(|v| !v.is_undefined());
            let Some(d) = field("value").or_else(|| field("default")) else {
                continue;
            };
            default = d;
            hint = field("hint").and_then(|h| convert::string(&h));
            tab = field("tab").and_then(|t| convert::string(&t));
            want_int = field("type")
                .and_then(|t| convert::string(&t))
                .is_some_and(|t| t.eq_ignore_ascii_case("int"));
        }
        let sv = match (want_int, convert::number(&default)) {
            (true, Some(n)) => Some(ScriptValue::Int(n.round() as i32)),
            _ => convert::to_script_value(&default, ctx),
        };
        if let Some(sv) = sv {
            out.push(VarDef {
                name,
                display_name,
                default_value: sv,
                hint,
                tab,
            });
        }
    }
    out
}

/// `move_speed` -> `Move Speed`, and `moveSpeed` -> `Move Speed` too, since
/// JavaScript names are usually camel case.
fn to_display_name(name: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in name.chars() {
        if ch == '_' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if ch.is_uppercase() && current.chars().last().is_some_and(|c| c.is_lowercase()) {
            words.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
