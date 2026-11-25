//! The engine's `ScriptContext`, flattened into the struct a C# hook reads.
//!
//! The Lua and Python plugins write some sixty variables into a script before
//! every hook; the JavaScript plugin turns each into a getter that reads the
//! context only when asked. C# gets the middle road: the values nearly every
//! script reads (time, transform, movement input, mouse, the first gamepad,
//! the entity) are copied once into a [`HookContext`] of plain numbers, which
//! the C# side reads as fields with no call at all, and everything else stays
//! behind a call (see [`crate::frame`]).
//!
//! Copying ~200 bytes per hook costs less than one call across the boundary,
//! so this is cheaper than a getter per value even for a script that reads
//! only `Delta`.

use bevy::math::EulerRot;
use renzora::ScriptContext;

use crate::abi::{flag, HookContext};

/// Fill a [`HookContext`] from `ctx`. `scratch` receives the joined timer
/// names and must outlive the call the context is passed to.
pub fn fill(ctx: &ScriptContext, scratch: &mut String) -> HookContext {
    scratch.clear();
    for (i, t) in ctx.timers_just_finished.iter().enumerate() {
        if i > 0 {
            scratch.push('\n');
        }
        scratch.push_str(t);
    }
    let t = &ctx.transform;
    // `XYZ` so x/y/z mean what their names say, as in the other plugins. In
    // degrees, the unit `SetRotation` and `Rotate` take: the engine's
    // rotation commands are degrees, and a script reading an angle back
    // should be able to hand it straight to them.
    let (ex, ey, ez) = t.rotation.to_euler(EulerRot::XYZ);
    let (ex, ey, ez) = (ex.to_degrees(), ey.to_degrees(), ez.to_degrees());
    let pad = ctx.gamepads.first();
    let mut pad_buttons = 0u32;
    if let Some(p) = pad {
        for (i, down) in p.buttons.iter().enumerate() {
            if *down {
                pad_buttons |= 1 << i;
            }
        }
    }
    let mut mouse_buttons = 0u32;
    for i in 0..5 {
        if ctx.mouse_buttons_pressed[i] {
            mouse_buttons |= 1 << i;
        }
        if ctx.mouse_buttons_just_pressed[i] {
            mouse_buttons |= 1 << (8 + i);
        }
    }
    let mut flags = 0;
    if ctx.has_parent {
        flags |= flag::HAS_PARENT;
    }
    if !ctx.active_collisions.is_empty() {
        flags |= flag::COLLIDING;
    }
    if ctx.net_is_server {
        flags |= flag::NET_SERVER;
    }
    if ctx.net_is_connected {
        flags |= flag::NET_CONNECTED;
    }
    let (name_ptr, name_len) = crate::abi::utf8(&ctx.self_entity_name);
    let (timers_ptr, timers_len) = crate::abi::utf8(scratch);
    HookContext {
        entity_id: ctx.self_entity_id,
        elapsed: ctx.time.elapsed,
        name_ptr,
        name_len,
        timers_ptr,
        timers_len,
        delta: ctx.time.delta,
        position: t.position.to_array(),
        rotation: t.rotation.to_array(),
        euler: [ex, ey, ez],
        scale: t.scale.to_array(),
        input: ctx.input_movement.to_array(),
        mouse: ctx.mouse_position.to_array(),
        mouse_delta: ctx.mouse_delta.to_array(),
        mouse_scroll: ctx.mouse_scroll,
        camera_yaw: ctx.camera_yaw,
        camera_ev: ctx.camera_ev,
        project_size: [ctx.project_width, ctx.project_height],
        pad_left: pad.map(|p| p.left_stick.to_array()).unwrap_or_default(),
        pad_right: pad.map(|p| p.right_stick.to_array()).unwrap_or_default(),
        pad_triggers: pad.map(|p| [p.left_trigger, p.right_trigger]).unwrap_or_default(),
        pad_buttons,
        mouse_buttons,
        flags,
        net_player_count: ctx.net_player_count,
        gamepad_count: ctx.gamepads.len() as u32,
        parent_position: ctx.parent_position.to_array(),
        health: ctx.self_health,
        max_health: ctx.self_max_health,
    }
}
