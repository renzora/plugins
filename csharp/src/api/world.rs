//! Physics, timers, the entity's look, the cursor and camera, spawning, scenes
//! and the environment: everything that is "push one command" with nothing to
//! read back.

use renzora::ScriptCommand;

use super::{push, Args, Out};

pub fn call(name: &str, a: &Args) -> Option<Out> {
    Some(match name {
        // -- Physics --
        "apply_force" => a.xyz(0).and_then(|force| push(ScriptCommand::ApplyForce { entity_id: None, force })),
        "apply_impulse" => a
            .xyz(0)
            .and_then(|impulse| push(ScriptCommand::ApplyImpulse { entity_id: None, impulse })),
        "set_velocity" => a
            .xyz(0)
            .and_then(|velocity| push(ScriptCommand::SetVelocity { entity_id: None, velocity })),
        "set_gravity_scale" => a
            .f32(0, "scale")
            .and_then(|scale| push(ScriptCommand::SetGravityScale { entity_id: None, scale })),

        // -- Timers: a finished one appears in `TimersFinished` next hook --
        "start_timer" => (|| {
            push(ScriptCommand::StartTimer {
                name: a.str(0, "name")?,
                duration: a.f32(1, "duration")?,
                repeat: a.opt_bool(2, "repeat")?.unwrap_or(false),
            })
        })(),
        "stop_timer" => a.str(0, "name").and_then(|name| push(ScriptCommand::StopTimer { name })),

        // -- Rendering --
        "set_visibility" => a
            .bool(0, "visible")
            .and_then(|visible| push(ScriptCommand::SetVisibility { entity_id: None, visible })),
        // set_material_color(color) or set_material_color(r, g, b, a = 1).
        "set_material_color" => (|| {
            let color = match a.opt_color(0) {
                Some(c) => c,
                None => [
                    a.f32(0, "r")?,
                    a.f32(1, "g")?,
                    a.f32(2, "b")?,
                    a.opt_f32(3, "a")?.unwrap_or(1.0),
                ],
            };
            push(ScriptCommand::SetMaterialColor { entity_id: None, color })
        })(),

        // -- Cursor and camera --
        "lock_cursor" => push(ScriptCommand::LockCursor),
        "unlock_cursor" => push(ScriptCommand::UnlockCursor),
        "screen_shake" => (|| {
            push(ScriptCommand::ScreenShake {
                intensity: a.f32(0, "intensity")?,
                duration: a.f32(1, "duration")?,
            })
        })(),

        // -- Entities --
        "spawn_entity" => a.str(0, "name").and_then(|name| push(ScriptCommand::SpawnEntity { name })),
        // spawn_primitive(name, kind, x, y, z, r?, g?, b?), or with a vector
        // position and a Color / Vector3 tint. Without a tint the shape keeps
        // its registered colour.
        "spawn_primitive" => (|| {
            let name = a.str(0, "name")?;
            let kind = a.str(1, "kind")?;
            let (position, used) = a.xyz_at(2)?;
            let at = 2 + used;
            let color = match a.opt_color(at) {
                Some(c) => Some(c),
                None => match (a.opt_f32(at, "r")?, a.opt_f32(at + 1, "g")?, a.opt_f32(at + 2, "b")?) {
                    (Some(r), Some(g), Some(b)) => Some([r, g, b, 1.0]),
                    _ => None,
                },
            };
            push(ScriptCommand::SpawnPrimitive {
                name,
                primitive_type: kind,
                position: Some(position),
                scale: None,
                color,
            })
        })(),
        "despawn_self" => push(ScriptCommand::DespawnSelf),
        // despawn_by_prefix("chunk_3_5_"): every entity whose Name starts with
        // the prefix, so a streaming script releases a chunk in one call.
        "despawn_by_prefix" => a
            .str(0, "prefix")
            .and_then(|prefix| push(ScriptCommand::DespawnByPrefix { prefix })),

        // -- Scenes and the environment --
        "load_scene" => a.str(0, "path").and_then(|path| push(ScriptCommand::LoadScene { path })),
        "set_sun_angles" => (|| {
            push(ScriptCommand::SetSunAngles {
                azimuth: a.f32(0, "azimuth")?,
                elevation: a.f32(1, "elevation")?,
            })
        })(),
        "set_fog" => (|| {
            push(ScriptCommand::SetFog {
                enabled: a.bool(0, "enabled")?,
                start: a.f32(1, "start")?,
                end: a.f32(2, "end")?,
            })
        })(),
        _ => return None,
    })
}
