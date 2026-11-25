//! Physics, timers, the entity's look, the cursor and camera, spawning, scenes
//! and the environment: everything that is "push one command" with nothing to
//! read back.

use boa_engine::{Context, JsObject};
use renzora::ScriptCommand;

use super::command;
use crate::convert::{self, Shape};

pub fn register(ctx: &mut Context, m: &JsObject) {
    physics(ctx, m);
    timers(ctx, m);
    rendering(ctx, m);
    cursor_and_camera(ctx, m);
    ecs(ctx, m);
    scene_and_environment(ctx, m);
}

/// Forces, impulses and the rigid body's own settings.
fn physics(ctx: &mut Context, m: &JsObject) {
    command!(ctx, m, "apply_force", |a, c| ScriptCommand::ApplyForce {
        entity_id: None,
        force: a.xyz(0, c)?,
    });
    command!(ctx, m, "apply_impulse", |a, c| ScriptCommand::ApplyImpulse {
        entity_id: None,
        impulse: a.xyz(0, c)?,
    });
    command!(ctx, m, "set_velocity", |a, c| ScriptCommand::SetVelocity {
        entity_id: None,
        velocity: a.xyz(0, c)?,
    });
    command!(ctx, m, "set_gravity_scale", |a, c| ScriptCommand::SetGravityScale {
        entity_id: None,
        scale: a.f32(0, "scale")?,
    });
}

/// Named timers the host counts down; a finished one appears in the
/// `timers_finished` array on the next hook.
fn timers(ctx: &mut Context, m: &JsObject) {
    // start_timer(name, duration, repeat = false)
    command!(ctx, m, "start_timer", |a, c| ScriptCommand::StartTimer {
        name: a.str(0, "name")?,
        duration: a.f32(1, "duration")?,
        repeat: a.opt_bool(2).unwrap_or(false),
    });
    command!(ctx, m, "stop_timer", |a, c| ScriptCommand::StopTimer {
        name: a.str(0, "name")?,
    });
}

/// Visibility and the material a mesh draws with.
fn rendering(ctx: &mut Context, m: &JsObject) {
    command!(ctx, m, "set_visibility", |a, c| ScriptCommand::SetVisibility {
        entity_id: None,
        visible: a.bool(0, "visible")?,
    });
    // set_material_color(r, g, b, a = 1), or set_material_color(color).
    command!(ctx, m, "set_material_color", |a, c| {
        let first = a.get(0).cloned();
        let color = match first.and_then(|v| convert::shape(&v, c)) {
            Some(Shape::Color(col)) => col,
            Some(Shape::Vec3([r, g, b])) => [r, g, b, 1.0],
            _ => [
                a.f32(0, "r")?,
                a.f32(1, "g")?,
                a.f32(2, "b")?,
                a.opt_f32(3, "a")?.unwrap_or(1.0),
            ],
        };
        ScriptCommand::SetMaterialColor {
            entity_id: None,
            color,
        }
    });
}

/// The mouse cursor and the active camera: one group because a script that
/// locks the cursor is nearly always the one driving the camera with it.
fn cursor_and_camera(ctx: &mut Context, m: &JsObject) {
    command!(ctx, m, "lock_cursor", |a, c| {
        let _ = a;
        ScriptCommand::LockCursor
    });
    command!(ctx, m, "unlock_cursor", |a, c| {
        let _ = a;
        ScriptCommand::UnlockCursor
    });
    command!(ctx, m, "screen_shake", |a, c| ScriptCommand::ScreenShake {
        intensity: a.f32(0, "intensity")?,
        duration: a.f32(1, "duration")?,
    });
}

/// Spawning, despawning and finding entities.
fn ecs(ctx: &mut Context, m: &JsObject) {
    command!(ctx, m, "spawn_entity", |a, c| ScriptCommand::SpawnEntity {
        name: a.str(0, "name")?,
    });
    // spawn_primitive(name, kind, x, y, z, r?, g?, b?)
    //   kind: "cube" | "sphere" | "wall" | ... (any id in ShapeRegistry)
    //   r/g/b: optional; without all three the shape keeps its registered tint.
    // The position may also be one vector, and the colour one Color or Vec3.
    command!(ctx, m, "spawn_primitive", |a, c| {
        let name = a.str(0, "name")?;
        let kind = a.str(1, "kind")?;
        let (position, used) = a.xyz_at(2, c)?;
        let at = 2 + used;
        let tint = a.get(at).cloned();
        let color = match tint.and_then(|v| convert::shape(&v, c)) {
            Some(Shape::Color(col)) => Some(col),
            Some(Shape::Vec3([r, g, b])) => Some([r, g, b, 1.0]),
            _ => match (a.opt_f32(at, "r")?, a.opt_f32(at + 1, "g")?, a.opt_f32(at + 2, "b")?) {
                (Some(r), Some(g), Some(b)) => Some([r, g, b, 1.0]),
                _ => None,
            },
        };
        ScriptCommand::SpawnPrimitive {
            name,
            primitive_type: kind,
            position: Some(position),
            scale: None,
            color,
        }
    });
    command!(ctx, m, "despawn_self", |a, c| {
        let _ = a;
        ScriptCommand::DespawnSelf
    });
    // despawn_by_prefix("chunk_3_5_"): every entity whose Name starts with the
    // prefix, so a streaming script releases a chunk in one call.
    command!(ctx, m, "despawn_by_prefix", |a, c| ScriptCommand::DespawnByPrefix {
        prefix: a.str(0, "prefix")?,
    });
}

/// Loading a different scene, and the sun, sky and fog.
fn scene_and_environment(ctx: &mut Context, m: &JsObject) {
    command!(ctx, m, "load_scene", |a, c| ScriptCommand::LoadScene {
        path: a.str(0, "path")?,
    });
    command!(ctx, m, "set_sun_angles", |a, c| ScriptCommand::SetSunAngles {
        azimuth: a.f32(0, "azimuth")?,
        elevation: a.f32(1, "elevation")?,
    });
    command!(ctx, m, "set_fog", |a, c| ScriptCommand::SetFog {
        enabled: a.bool(0, "enabled")?,
        start: a.f32(1, "start")?,
        end: a.f32(2, "end")?,
    });
}
