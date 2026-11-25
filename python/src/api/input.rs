//! Keyboard, the named action map, gamepads and touch.
//!
//! Every function here reads the running hook's [`ScriptContext`] through
//! [`frame::with`] instead of a dictionary copied into the script's globals
//! (which is what the Lua plugin does with its `_keys_pressed` tables). Outside
//! a hook nothing is pressed and every axis reads zero.
//!
//! Where Lua returns two values (`local x, y = gamepad_left_stick(0)`), these
//! return a `vec2`, which unpacks the same way: `x, y = gamepad_left_stick(0)`.
//!
//! Touch fingers and taps are indexed from **0**, the Python convention, where
//! Lua counts from 1.

use renzora::{GamepadSnapshot, ScriptContext, GAMEPAD_BUTTON_NAMES};
use rustpython_vm::builtins::PyDict;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::{Py, PyResult, VirtualMachine};

use super::add;
use super::args::Args;
use crate::convert;
use crate::frame;

/// A pressed-state map lookup, `false` when absent or outside a hook.
fn flag(map: impl FnOnce(&ScriptContext) -> Option<bool>) -> bool {
    frame::with(map).flatten().unwrap_or(false)
}

fn pad<R>(slot: i64, f: impl FnOnce(&GamepadSnapshot) -> R) -> Option<R> {
    frame::with(|ctx| ctx.gamepads.iter().find(|p| p.id as i64 == slot).map(f)).flatten()
}

fn button_index(name: &str) -> Option<usize> {
    GAMEPAD_BUTTON_NAMES.iter().position(|n| *n == name)
}

pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    keyboard(vm, m);
    actions(vm, m);
    gamepads(vm, m);
    touch(vm, m);
}

fn keyboard(vm: &VirtualMachine, m: &Py<PyDict>) {
    add(vm, m, "is_key_pressed", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let key = Args::new(&args, vm, "is_key_pressed").str(0, "key")?;
        Ok(flag(|c| c.keys_pressed.get(&key).copied()))
    });
    add(vm, m, "is_key_just_pressed", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let key = Args::new(&args, vm, "is_key_just_pressed").str(0, "key")?;
        Ok(flag(|c| c.keys_just_pressed.get(&key).copied()))
    });
    add(vm, m, "is_key_just_released", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let key = Args::new(&args, vm, "is_key_just_released").str(0, "key")?;
        Ok(flag(|c| c.keys_just_released.get(&key).copied()))
    });
}

/// The InputMap's actions, by name, so a script reads "jump" rather than a key
/// and works the same on keyboard and gamepad.
fn actions(vm: &VirtualMachine, m: &Py<PyDict>) {
    add(vm, m, "input_button_pressed", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let name = Args::new(&args, vm, "input_button_pressed").str(0, "name")?;
        Ok(flag(|c| c.action_pressed.get(&name).copied()))
    });
    add(vm, m, "input_button_just_pressed", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let name = Args::new(&args, vm, "input_button_just_pressed").str(0, "name")?;
        Ok(flag(|c| c.action_just_pressed.get(&name).copied()))
    });
    add(vm, m, "input_button_just_released", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let name = Args::new(&args, vm, "input_button_just_released").str(0, "name")?;
        Ok(flag(|c| c.action_just_released.get(&name).copied()))
    });
    add(vm, m, "input_axis_1d", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<f64> {
        let name = Args::new(&args, vm, "input_axis_1d").str(0, "name")?;
        Ok(frame::with(|c| c.action_axis_1d.get(&name).copied())
            .flatten()
            .unwrap_or(0.0) as f64)
    });
    add(vm, m, "input_axis_2d", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let name = Args::new(&args, vm, "input_axis_2d").str(0, "name")?;
        let v = frame::with(|c| c.action_axis_2d.get(&name).copied())
            .flatten()
            .unwrap_or_default();
        convert::vec2(vm, v.x, v.y)
    });
}

/// Pads by stable slot id (0 = first pad). The `gamepad_left_x`-style
/// variables mirror the first connected pad for older scripts.
fn gamepads(vm: &VirtualMachine, m: &Py<PyDict>) {
    add(vm, m, "gamepad_count", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<i64> {
        Ok(frame::with(|c| c.gamepads.len() as i64).unwrap_or(0))
    });
    add(vm, m, "gamepad_connected", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let slot = Args::new(&args, vm, "gamepad_connected").i64(0, "pad")?;
        Ok(pad(slot, |_| ()).is_some())
    });
    // Axis names: "left_x", "left_y", "right_x", "right_y", "left_trigger",
    // "right_trigger".
    add(vm, m, "gamepad_axis", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<f64> {
        let a = Args::new(&args, vm, "gamepad_axis");
        let slot = a.i64(0, "pad")?;
        let name = a.str(1, "axis")?;
        Ok(pad(slot, |p| match name.as_str() {
            "left_x" => p.left_stick.x,
            "left_y" => p.left_stick.y,
            "right_x" => p.right_stick.x,
            "right_y" => p.right_stick.y,
            "left_trigger" => p.left_trigger,
            "right_trigger" => p.right_trigger,
            _ => 0.0,
        })
        .unwrap_or(0.0) as f64)
    });
    add(vm, m, "gamepad_left_stick", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let slot = Args::new(&args, vm, "gamepad_left_stick").i64(0, "pad")?;
        let v = pad(slot, |p| p.left_stick).unwrap_or_default();
        convert::vec2(vm, v.x, v.y)
    });
    add(vm, m, "gamepad_right_stick", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let slot = Args::new(&args, vm, "gamepad_right_stick").i64(0, "pad")?;
        let v = pad(slot, |p| p.right_stick).unwrap_or_default();
        convert::vec2(vm, v.x, v.y)
    });
    // Button names: "south", "east", "west", "north", "l1", "r1", "l2", "r2",
    // "select", "start", "l3", "r3", "dpad_up", "dpad_down", "dpad_left",
    // "dpad_right".
    add(vm, m, "gamepad_button", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let a = Args::new(&args, vm, "gamepad_button");
        let slot = a.i64(0, "pad")?;
        let Some(i) = button_index(&a.str(1, "button")?) else {
            return Ok(false);
        };
        Ok(pad(slot, |p| p.buttons[i]).unwrap_or(false))
    });
    add(vm, m, "gamepad_button_just_pressed", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let a = Args::new(&args, vm, "gamepad_button_just_pressed");
        let slot = a.i64(0, "pad")?;
        let Some(i) = button_index(&a.str(1, "button")?) else {
            return Ok(false);
        };
        Ok(pad(slot, |p| p.buttons_just_pressed[i]).unwrap_or(false))
    });
}

/// Touch screens: every finger, plus taps and two-finger pinch and twist.
///
/// Fingers are read by position, `0` to `touch_count() - 1`, ordered by when
/// they went down. That index shifts as fingers lift, so a script following one
/// finger across frames keeps its `touch_id(i)` instead.
fn touch(vm: &VirtualMachine, m: &Py<PyDict>) {
    // The finger's fields through `read`, or `None` for an index with no finger.
    // Takes the field reader rather than handing out the finger itself, so the
    // contract's touch type need not be named here.
    macro_rules! finger {
        ($i:expr, |$t:ident| $read:expr) => {
            usize::try_from($i)
                .ok()
                .and_then(|i| frame::with(|c| c.touch.touches.get(i).map(|$t| $read)).flatten())
        };
    }

    add(vm, m, "touch_count", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<i64> {
        Ok(frame::with(|c| c.touch.touches.len() as i64).unwrap_or(0))
    });
    // `None` for an index with no finger, as Lua returns `nil`.
    add(vm, m, "touch_id", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<Option<i64>> {
        let i = Args::new(&args, vm, "touch_id").i64(0, "index")?;
        // Ids come from the OS as u64; no platform hands out one near the top bit.
        Ok(finger!(i, |t| t.id as i64))
    });
    add(vm, m, "touch_position", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let i = Args::new(&args, vm, "touch_position").i64(0, "index")?;
        let v = finger!(i, |t| t.position).unwrap_or_default();
        convert::vec2(vm, v.x, v.y)
    });
    add(vm, m, "touch_start", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let i = Args::new(&args, vm, "touch_start").i64(0, "index")?;
        let v = finger!(i, |t| t.start_position).unwrap_or_default();
        convert::vec2(vm, v.x, v.y)
    });
    add(vm, m, "touch_delta", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let i = Args::new(&args, vm, "touch_delta").i64(0, "index")?;
        let v = finger!(i, |t| t.delta).unwrap_or_default();
        convert::vec2(vm, v.x, v.y)
    });
    add(vm, m, "touch_just_pressed", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let i = Args::new(&args, vm, "touch_just_pressed").i64(0, "index")?;
        Ok(finger!(i, |t| t.just_pressed).unwrap_or(false))
    });
    add(vm, m, "touch_just_released", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<bool> {
        let i = Args::new(&args, vm, "touch_just_released").i64(0, "index")?;
        Ok(finger!(i, |t| t.just_released).unwrap_or(false))
    });
    add(vm, m, "pinch_delta", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<f64> {
        Ok(frame::with(|c| c.touch.pinch as f64).unwrap_or(0.0))
    });
    add(vm, m, "rotate_delta", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<f64> {
        Ok(frame::with(|c| c.touch.rotation as f64).unwrap_or(0.0))
    });
    add(vm, m, "tap_count", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<i64> {
        Ok(frame::with(|c| c.touch.taps.len() as i64).unwrap_or(0))
    });
    add(vm, m, "tap_position", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let i = Args::new(&args, vm, "tap_position").i64(0, "index")?;
        let v = usize::try_from(i)
            .ok()
            .and_then(|i| frame::with(|c| c.touch.taps.get(i).copied()).flatten())
            .unwrap_or_default();
        convert::vec2(vm, v.x, v.y)
    });
}
