//! The per-call variables a script reads (`delta`, `position_x`, ...), and its
//! props.
//!
//! Set as globals in the script's own namespace before every hook, under the
//! Lua plugin's names, so `translate(input_x * speed * delta, 0, 0)` reads the
//! same in both languages. They are inputs: assigning `position_x = 3` changes
//! the variable and nothing else, which is what the starter script warns about.
//!
//! What Lua also copies in (the key maps, the action map, every gamepad and
//! finger) is NOT copied here; the functions that read those go to the context
//! directly (see `crate::frame`).

use bevy::math::EulerRot;
use renzora::{ScriptContext, ScriptValue, ScriptVariableDefinition as VarDef, GAMEPAD_BUTTON_NAMES};
use rustpython_vm::builtins::{PyDict, PyDictRef};
use rustpython_vm::{Py, PyObjectRef, PyResult, VirtualMachine};

use crate::convert;

/// Overwrite the context variables (and, when given, the script's props) in
/// `g`.
///
/// Done in place every call, so the cost scales with the context's size
/// rather than with the API's. Props come last so an inspector value wins over
/// whatever the script's top level assigned, as in Lua.
pub fn set_globals(
    vm: &VirtualMachine,
    g: &Py<PyDict>,
    ctx: &ScriptContext,
    vars: &[(String, ScriptValue)],
) -> PyResult<()> {
    let f = |k: &str, v: f64| g.set_item(k, convert::float(vm, v), vm);
    let b = |k: &str, v: bool| g.set_item(k, convert::boolean(vm, v), vm);

    // Time
    f("delta", ctx.time.delta as f64)?;
    f("elapsed", ctx.time.elapsed)?;

    // Transform. `XYZ` so x/y/z below mean what their names say.
    let (rx, ry, rz) = ctx.transform.rotation.to_euler(EulerRot::XYZ);
    f("position_x", ctx.transform.position.x as f64)?;
    f("position_y", ctx.transform.position.y as f64)?;
    f("position_z", ctx.transform.position.z as f64)?;
    f("rotation_x", rx as f64)?;
    f("rotation_y", ry as f64)?;
    f("rotation_z", rz as f64)?;
    f("scale_x", ctx.transform.scale.x as f64)?;
    f("scale_y", ctx.transform.scale.y as f64)?;
    f("scale_z", ctx.transform.scale.z as f64)?;

    // Input
    f("input_x", ctx.input_movement.x as f64)?;
    f("input_y", ctx.input_movement.y as f64)?;
    f("mouse_x", ctx.mouse_position.x as f64)?;
    f("mouse_y", ctx.mouse_position.y as f64)?;
    f("mouse_delta_x", ctx.mouse_delta.x as f64)?;
    f("mouse_delta_y", ctx.mouse_delta.y as f64)?;
    f("camera_yaw", ctx.camera_yaw as f64)?;
    b("mouse_left", ctx.mouse_buttons_pressed[0])?;
    b("mouse_right", ctx.mouse_buttons_pressed[1])?;
    b("mouse_middle", ctx.mouse_buttons_pressed[2])?;
    b("mouse_left_just_pressed", ctx.mouse_buttons_just_pressed[0])?;
    b("mouse_right_just_pressed", ctx.mouse_buttons_just_pressed[1])?;
    f("mouse_scroll", ctx.mouse_scroll as f64)?;

    // Camera exposure (live scene EV-100) and the configured game resolution.
    f("camera_ev", ctx.camera_ev as f64)?;
    f("project_width", ctx.project_width as f64)?;
    f("project_height", ctx.project_height as f64)?;

    // The first connected gamepad, for scripts written before multi-pad.
    let first = ctx.gamepads.first();
    let axis = |read: fn(&renzora::GamepadSnapshot) -> f32| first.map(read).unwrap_or(0.0) as f64;
    f("gamepad_left_x", axis(|p| p.left_stick.x))?;
    f("gamepad_left_y", axis(|p| p.left_stick.y))?;
    f("gamepad_right_x", axis(|p| p.right_stick.x))?;
    f("gamepad_right_y", axis(|p| p.right_stick.y))?;
    f("gamepad_left_trigger", axis(|p| p.left_trigger))?;
    f("gamepad_right_trigger", axis(|p| p.right_trigger))?;
    let buttons = first.map(|p| p.buttons).unwrap_or([false; 16]);
    for (i, name) in GAMEPAD_BUTTON_NAMES.iter().enumerate() {
        b(&format!("gamepad_{name}"), buttons[i])?;
    }

    // Entity
    g.set_item("self_entity_id", convert::int(vm, ctx.self_entity_id as i64), vm)?;
    g.set_item("self_entity_name", convert::text(vm, &ctx.self_entity_name), vm)?;
    b("is_colliding", !ctx.active_collisions.is_empty())?;
    g.set_item(
        "timers_finished",
        convert::text_list(vm, &ctx.timers_just_finished),
        vm,
    )?;
    f("self_health", ctx.self_health as f64)?;
    f("self_max_health", ctx.self_max_health as f64)?;

    // Parent
    b("has_parent", ctx.has_parent)?;
    f("parent_position_x", ctx.parent_position.x as f64)?;
    f("parent_position_y", ctx.parent_position.y as f64)?;
    f("parent_position_z", ctx.parent_position.z as f64)?;

    // Props
    for (key, value) in vars {
        g.set_item(key.as_str(), convert::from_script_value(vm, value)?, vm)?;
    }
    Ok(())
}

/// Read the script's props back out of its globals after a hook.
///
/// Only the names the inspector already knows: a script assigning a fresh
/// global is not promoted to a prop, which keeps the inspector's rows stable.
pub fn read_back(vm: &VirtualMachine, g: &Py<PyDict>, vars: &[(String, ScriptValue)]) -> Vec<(String, ScriptValue)> {
    let mut out = Vec::with_capacity(vars.len());
    for (name, _) in vars {
        if let Ok(Some(value)) = g.get_item_opt(name.as_str(), vm) {
            if let Some(sv) = convert::to_script_value(&value, vm) {
                out.push((name.clone(), sv));
            }
        }
    }
    out
}

/// The props a script declares in `props()`.
///
/// `props()` returns a dict of name to either a bare default (`{"speed":
/// 5.0}`) or a dict carrying `value` (or `default`) plus optional `hint` and
/// `tab` (`{"speed": {"value": 5.0, "hint": "metres per second"}}`), the same
/// two forms the Lua plugin reads from a table.
pub fn props(vm: &VirtualMachine, globals: &PyDictRef) -> Vec<VarDef> {
    let mut out = Vec::new();
    let Ok(Some(func)) = globals.get_item_opt("props", vm) else {
        return out;
    };
    if !func.is_callable() {
        return out;
    }
    let Ok(result) = func.call((), vm) else {
        return out;
    };
    let Some(table) = result.downcast_ref::<PyDict>() else {
        return out;
    };
    for (key, value) in table {
        let Some(name) = convert::string(&key) else {
            continue;
        };
        let display_name = to_display_name(&name);
        if let Some(spec) = value.downcast_ref::<PyDict>() {
            let field = |k: &str| spec.get_item_opt(k, vm).ok().flatten();
            let default: Option<PyObjectRef> = field("value").or_else(|| field("default"));
            if let Some(sv) = default.and_then(|d| convert::to_script_value(&d, vm)) {
                out.push(VarDef {
                    name,
                    display_name,
                    default_value: sv,
                    hint: field("hint").and_then(|h| convert::string(&h)),
                    tab: field("tab").and_then(|t| convert::string(&t)),
                });
                continue;
            }
        }
        if let Some(sv) = convert::to_script_value(&value, vm) {
            out.push(VarDef {
                name,
                display_name,
                default_value: sv,
                hint: None,
                tab: None,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `move_speed` -> `Move Speed`, for the inspector row.
fn to_display_name(name: &str) -> String {
    name.split('_')
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
