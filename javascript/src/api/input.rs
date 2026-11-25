//! Keyboard, the named action map, gamepads and touch.
//!
//! Every function here reads the running hook's [`ScriptContext`] through
//! [`frame::with`] (see that module for why nothing is copied into the
//! interpreter). Outside a hook nothing is pressed and every axis reads zero.
//!
//! Where Lua returns two values (`local x, y = gamepad_left_stick(0)`), these
//! return a `Vec2`, which reads as `.x` / `.y` and also destructures:
//! `const [x, y] = gamepad_left_stick(0)`.
//!
//! Touch fingers and taps are indexed from **0**, the JavaScript convention,
//! where Lua counts from 1.

use boa_engine::{Context, JsObject, JsValue};
use renzora::{GamepadSnapshot, ScriptContext, GAMEPAD_BUTTON_NAMES};

use super::add;
use super::args::Args;
use crate::convert;
use crate::frame;

/// A pressed-state map lookup, `false` when absent or outside a hook.
fn flag(map: impl FnOnce(&ScriptContext) -> Option<bool>) -> JsValue {
    JsValue::from(frame::with(map).flatten().unwrap_or(false))
}

fn pad<R>(slot: i64, f: impl FnOnce(&GamepadSnapshot) -> R) -> Option<R> {
    frame::with(|ctx| ctx.gamepads.iter().find(|p| p.id as i64 == slot).map(f)).flatten()
}

fn button_index(name: &str) -> Option<usize> {
    GAMEPAD_BUTTON_NAMES.iter().position(|n| *n == name)
}

pub fn register(ctx: &mut Context, m: &JsObject) {
    keyboard(ctx, m);
    actions(ctx, m);
    gamepads(ctx, m);
    touch(ctx, m);
}

fn keyboard(ctx: &mut Context, m: &JsObject) {
    add(ctx, m, "is_key_pressed", |_, args, _| {
        let key = Args::new(args, "is_key_pressed").str(0, "key")?;
        Ok(flag(|c| c.keys_pressed.get(&key).copied()))
    });
    add(ctx, m, "is_key_just_pressed", |_, args, _| {
        let key = Args::new(args, "is_key_just_pressed").str(0, "key")?;
        Ok(flag(|c| c.keys_just_pressed.get(&key).copied()))
    });
    add(ctx, m, "is_key_just_released", |_, args, _| {
        let key = Args::new(args, "is_key_just_released").str(0, "key")?;
        Ok(flag(|c| c.keys_just_released.get(&key).copied()))
    });
}

/// The InputMap's actions, by name, so a script reads "jump" rather than a key
/// and works the same on keyboard and gamepad.
fn actions(ctx: &mut Context, m: &JsObject) {
    add(ctx, m, "input_button_pressed", |_, args, _| {
        let name = Args::new(args, "input_button_pressed").str(0, "name")?;
        Ok(flag(|c| c.action_pressed.get(&name).copied()))
    });
    add(ctx, m, "input_button_just_pressed", |_, args, _| {
        let name = Args::new(args, "input_button_just_pressed").str(0, "name")?;
        Ok(flag(|c| c.action_just_pressed.get(&name).copied()))
    });
    add(ctx, m, "input_button_just_released", |_, args, _| {
        let name = Args::new(args, "input_button_just_released").str(0, "name")?;
        Ok(flag(|c| c.action_just_released.get(&name).copied()))
    });
    add(ctx, m, "input_axis_1d", |_, args, _| {
        let name = Args::new(args, "input_axis_1d").str(0, "name")?;
        let v = frame::with(|c| c.action_axis_1d.get(&name).copied())
            .flatten()
            .unwrap_or(0.0);
        Ok(JsValue::from(v as f64))
    });
    add(ctx, m, "input_axis_2d", |_, args, ctx| {
        let name = Args::new(args, "input_axis_2d").str(0, "name")?;
        let v = frame::with(|c| c.action_axis_2d.get(&name).copied())
            .flatten()
            .unwrap_or_default();
        convert::vec2(ctx, v.x, v.y)
    });
}

/// Pads by stable slot id (0 = first pad). The `gamepad_left_x`-style
/// variables mirror the first connected pad for older scripts.
fn gamepads(ctx: &mut Context, m: &JsObject) {
    add(ctx, m, "gamepad_count", |_, _, _| {
        Ok(JsValue::from(frame::with(|c| c.gamepads.len() as f64).unwrap_or(0.0)))
    });
    add(ctx, m, "gamepad_connected", |_, args, _| {
        let slot = Args::new(args, "gamepad_connected").i64(0, "pad")?;
        Ok(JsValue::from(pad(slot, |_| ()).is_some()))
    });
    // Axis names: "left_x", "left_y", "right_x", "right_y", "left_trigger",
    // "right_trigger".
    add(ctx, m, "gamepad_axis", |_, args, _| {
        let a = Args::new(args, "gamepad_axis");
        let slot = a.i64(0, "pad")?;
        let name = a.str(1, "axis")?;
        let v = pad(slot, |p| match name.as_str() {
            "left_x" => p.left_stick.x,
            "left_y" => p.left_stick.y,
            "right_x" => p.right_stick.x,
            "right_y" => p.right_stick.y,
            "left_trigger" => p.left_trigger,
            "right_trigger" => p.right_trigger,
            _ => 0.0,
        })
        .unwrap_or(0.0);
        Ok(JsValue::from(v as f64))
    });
    add(ctx, m, "gamepad_left_stick", |_, args, ctx| {
        let slot = Args::new(args, "gamepad_left_stick").i64(0, "pad")?;
        let v = pad(slot, |p| p.left_stick).unwrap_or_default();
        convert::vec2(ctx, v.x, v.y)
    });
    add(ctx, m, "gamepad_right_stick", |_, args, ctx| {
        let slot = Args::new(args, "gamepad_right_stick").i64(0, "pad")?;
        let v = pad(slot, |p| p.right_stick).unwrap_or_default();
        convert::vec2(ctx, v.x, v.y)
    });
    // Button names: "south", "east", "west", "north", "l1", "r1", "l2", "r2",
    // "select", "start", "l3", "r3", "dpad_up", "dpad_down", "dpad_left",
    // "dpad_right".
    add(ctx, m, "gamepad_button", |_, args, _| {
        let a = Args::new(args, "gamepad_button");
        let slot = a.i64(0, "pad")?;
        let Some(i) = button_index(&a.str(1, "button")?) else {
            return Ok(JsValue::from(false));
        };
        Ok(JsValue::from(pad(slot, |p| p.buttons[i]).unwrap_or(false)))
    });
    add(ctx, m, "gamepad_button_just_pressed", |_, args, _| {
        let a = Args::new(args, "gamepad_button_just_pressed");
        let slot = a.i64(0, "pad")?;
        let Some(i) = button_index(&a.str(1, "button")?) else {
            return Ok(JsValue::from(false));
        };
        Ok(JsValue::from(pad(slot, |p| p.buttons_just_pressed[i]).unwrap_or(false)))
    });
}

/// Touch screens: every finger, plus taps and two-finger pinch and twist.
///
/// Fingers are read by position, `0` to `touch_count() - 1`, ordered by when
/// they went down. That index shifts as fingers lift, so a script following one
/// finger across frames keeps its `touch_id(i)` instead.
fn touch(ctx: &mut Context, m: &JsObject) {
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

    add(ctx, m, "touch_count", |_, _, _| {
        Ok(JsValue::from(frame::with(|c| c.touch.touches.len() as f64).unwrap_or(0.0)))
    });
    // `null` for an index with no finger, as Lua returns `nil`.
    add(ctx, m, "touch_id", |_, args, _| {
        let i = Args::new(args, "touch_id").i64(0, "index")?;
        // Ids come from the OS as u64; no platform hands out one past 2^53.
        Ok(finger!(i, |t| t.id as f64).map(JsValue::from).unwrap_or_else(JsValue::null))
    });
    add(ctx, m, "touch_position", |_, args, ctx| {
        let i = Args::new(args, "touch_position").i64(0, "index")?;
        let v = finger!(i, |t| t.position).unwrap_or_default();
        convert::vec2(ctx, v.x, v.y)
    });
    add(ctx, m, "touch_start", |_, args, ctx| {
        let i = Args::new(args, "touch_start").i64(0, "index")?;
        let v = finger!(i, |t| t.start_position).unwrap_or_default();
        convert::vec2(ctx, v.x, v.y)
    });
    add(ctx, m, "touch_delta", |_, args, ctx| {
        let i = Args::new(args, "touch_delta").i64(0, "index")?;
        let v = finger!(i, |t| t.delta).unwrap_or_default();
        convert::vec2(ctx, v.x, v.y)
    });
    add(ctx, m, "touch_just_pressed", |_, args, _| {
        let i = Args::new(args, "touch_just_pressed").i64(0, "index")?;
        Ok(JsValue::from(finger!(i, |t| t.just_pressed).unwrap_or(false)))
    });
    add(ctx, m, "touch_just_released", |_, args, _| {
        let i = Args::new(args, "touch_just_released").i64(0, "index")?;
        Ok(JsValue::from(finger!(i, |t| t.just_released).unwrap_or(false)))
    });
    add(ctx, m, "pinch_delta", |_, _, _| {
        Ok(JsValue::from(frame::with(|c| c.touch.pinch as f64).unwrap_or(0.0)))
    });
    add(ctx, m, "rotate_delta", |_, _, _| {
        Ok(JsValue::from(frame::with(|c| c.touch.rotation as f64).unwrap_or(0.0)))
    });
    add(ctx, m, "tap_count", |_, _, _| {
        Ok(JsValue::from(frame::with(|c| c.touch.taps.len() as f64).unwrap_or(0.0)))
    });
    add(ctx, m, "tap_position", |_, args, ctx| {
        let i = Args::new(args, "tap_position").i64(0, "index")?;
        let v = usize::try_from(i)
            .ok()
            .and_then(|i| frame::with(|c| c.touch.taps.get(i).copied()).flatten())
            .unwrap_or_default();
        convert::vec2(ctx, v.x, v.y)
    });
}
