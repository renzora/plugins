//! Keyboard, the named action map, gamepads and touch, by name.
//!
//! Every read answers from the running hook's `ScriptContext` through
//! [`crate::frame::with`]; outside a hook nothing is pressed and every axis
//! reads zero. Keys and actions have a fast path of their own in the native
//! table (`IsKeyPressed` is read in loops); these are their by-name twins, plus
//! everything read too rarely to need one.
//!
//! Touch fingers and taps are indexed from **0**, the C# convention, as in the
//! JavaScript plugin (Lua counts from 1). Gamepads are by stable slot id, 0
//! being the first pad.

use renzora::serde_json::Value;
use renzora::{GamepadSnapshot, ScriptContext, GAMEPAD_BUTTON_NAMES};

use super::{Args, Out};
use crate::convert;
use crate::frame;

/// A pressed-state map lookup, `false` when absent or outside a hook.
fn flag(map: impl FnOnce(&ScriptContext) -> Option<bool>) -> Value {
    Value::Bool(frame::with(map).flatten().unwrap_or(false))
}

fn pad<R>(slot: i64, f: impl FnOnce(&GamepadSnapshot) -> R) -> Option<R> {
    frame::with(|ctx| ctx.gamepads.iter().find(|p| p.id as i64 == slot).map(f)).flatten()
}

fn button_index(name: &str) -> Option<usize> {
    GAMEPAD_BUTTON_NAMES.iter().position(|n| *n == name)
}

fn num(f: f32) -> Value {
    Value::from(f as f64)
}

/// The finger at index `i`, read through `read`.
fn finger<R>(i: i64, read: impl FnOnce(&renzora::context::TouchSnapshot) -> R) -> Option<R> {
    let i = usize::try_from(i).ok()?;
    frame::with(|c| c.touch.touches.get(i).map(read)).flatten()
}

pub fn call(name: &str, a: &Args) -> Option<Out> {
    let key = |field: fn(&ScriptContext) -> &std::collections::HashMap<String, bool>| -> Out {
        let k = a.str(0, "key")?;
        Ok(flag(|c| field(c).get(&k).copied()))
    };
    let action = |field: fn(&ScriptContext) -> &std::collections::HashMap<String, bool>| -> Out {
        let n = a.str(0, "name")?;
        Ok(flag(|c| field(c).get(&n).copied()))
    };
    Some(match name {
        "is_key_pressed" => key(|c| &c.keys_pressed),
        "is_key_just_pressed" => key(|c| &c.keys_just_pressed),
        "is_key_just_released" => key(|c| &c.keys_just_released),
        "input_button_pressed" => action(|c| &c.action_pressed),
        "input_button_just_pressed" => action(|c| &c.action_just_pressed),
        "input_button_just_released" => action(|c| &c.action_just_released),
        "input_axis_1d" => a.str(0, "name").map(|n| {
            num(frame::with(|c| c.action_axis_1d.get(&n).copied()).flatten().unwrap_or(0.0))
        }),
        "input_axis_2d" => a.str(0, "name").map(|n| {
            let v = frame::with(|c| c.action_axis_2d.get(&n).copied()).flatten().unwrap_or_default();
            convert::vec2(v.x, v.y)
        }),

        "gamepad_count" => Ok(Value::from(frame::with(|c| c.gamepads.len()).unwrap_or(0))),
        "gamepad_connected" => a.i64(0, "pad").map(|slot| Value::Bool(pad(slot, |_| ()).is_some())),
        // Axis names: "left_x", "left_y", "right_x", "right_y", "left_trigger",
        // "right_trigger".
        "gamepad_axis" => (|| {
            let slot = a.i64(0, "pad")?;
            let axis = a.str(1, "axis")?;
            let v = pad(slot, |p| match axis.as_str() {
                "left_x" => p.left_stick.x,
                "left_y" => p.left_stick.y,
                "right_x" => p.right_stick.x,
                "right_y" => p.right_stick.y,
                "left_trigger" => p.left_trigger,
                "right_trigger" => p.right_trigger,
                _ => 0.0,
            });
            Ok(num(v.unwrap_or(0.0)))
        })(),
        "gamepad_left_stick" => a.i64(0, "pad").map(|slot| {
            let v = pad(slot, |p| p.left_stick).unwrap_or_default();
            convert::vec2(v.x, v.y)
        }),
        "gamepad_right_stick" => a.i64(0, "pad").map(|slot| {
            let v = pad(slot, |p| p.right_stick).unwrap_or_default();
            convert::vec2(v.x, v.y)
        }),
        // Button names: "south", "east", "west", "north", "l1", "r1", "l2",
        // "r2", "select", "start", "l3", "r3", "dpad_up", "dpad_down",
        // "dpad_left", "dpad_right".
        "gamepad_button" | "gamepad_button_just_pressed" => (|| {
            let slot = a.i64(0, "pad")?;
            let Some(i) = button_index(&a.str(1, "button")?) else {
                return Ok(Value::Bool(false));
            };
            let just = name == "gamepad_button_just_pressed";
            Ok(Value::Bool(
                pad(slot, |p| if just { p.buttons_just_pressed[i] } else { p.buttons[i] }).unwrap_or(false),
            ))
        })(),

        "touch_count" => Ok(Value::from(frame::with(|c| c.touch.touches.len()).unwrap_or(0))),
        // `null` for an index with no finger.
        "touch_id" => a
            .i64(0, "index")
            .map(|i| finger(i, |t| Value::from(t.id)).unwrap_or(Value::Null)),
        "touch_position" => a.i64(0, "index").map(|i| {
            let v = finger(i, |t| t.position).unwrap_or_default();
            convert::vec2(v.x, v.y)
        }),
        "touch_start" => a.i64(0, "index").map(|i| {
            let v = finger(i, |t| t.start_position).unwrap_or_default();
            convert::vec2(v.x, v.y)
        }),
        "touch_delta" => a.i64(0, "index").map(|i| {
            let v = finger(i, |t| t.delta).unwrap_or_default();
            convert::vec2(v.x, v.y)
        }),
        "touch_just_pressed" => a
            .i64(0, "index")
            .map(|i| Value::Bool(finger(i, |t| t.just_pressed).unwrap_or(false))),
        "touch_just_released" => a
            .i64(0, "index")
            .map(|i| Value::Bool(finger(i, |t| t.just_released).unwrap_or(false))),
        "pinch_delta" => Ok(num(frame::with(|c| c.touch.pinch).unwrap_or(0.0))),
        "rotate_delta" => Ok(num(frame::with(|c| c.touch.rotation).unwrap_or(0.0))),
        "tap_count" => Ok(Value::from(frame::with(|c| c.touch.taps.len()).unwrap_or(0))),
        "tap_position" => a.i64(0, "index").map(|i| {
            let v = usize::try_from(i)
                .ok()
                .and_then(|i| frame::with(|c| c.touch.taps.get(i).copied()).flatten())
                .unwrap_or_default();
            convert::vec2(v.x, v.y)
        }),

        // -- Network status, which lives in the same context --
        "net_is_server" => Ok(Value::Bool(frame::with(|c| c.net_is_server).unwrap_or(false))),
        "net_is_client" => Ok(Value::Bool(
            frame::with(|c| c.net_is_connected && !c.net_is_server).unwrap_or(false),
        )),
        "net_is_connected" => Ok(Value::Bool(frame::with(|c| c.net_is_connected).unwrap_or(false))),
        "net_player_count" => Ok(Value::from(frame::with(|c| c.net_player_count).unwrap_or(0))),
        _ => return None,
    })
}
