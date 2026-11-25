//! Physics, timers, the entity's look, the cursor and camera, spawning, scenes
//! and the environment: everything that is "push one command" with nothing to
//! read back.

use renzora::ScriptCommand;
use rustpython_vm::builtins::PyDict;
use rustpython_vm::{Py, VirtualMachine};

use super::command;

pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    physics(vm, m);
    timers(vm, m);
    rendering(vm, m);
    cursor_and_camera(vm, m);
    ecs(vm, m);
    scene_and_environment(vm, m);
}

/// Forces, impulses and the rigid body's own settings.
fn physics(vm: &VirtualMachine, m: &Py<PyDict>) {
    command!(vm, m, "apply_force", |a| ScriptCommand::ApplyForce {
        entity_id: None,
        force: a.xyz(0)?,
    });
    command!(vm, m, "apply_impulse", |a| ScriptCommand::ApplyImpulse {
        entity_id: None,
        impulse: a.xyz(0)?,
    });
    command!(vm, m, "set_velocity", |a| ScriptCommand::SetVelocity {
        entity_id: None,
        velocity: a.xyz(0)?,
    });
    command!(vm, m, "set_gravity_scale", |a| ScriptCommand::SetGravityScale {
        entity_id: None,
        scale: a.f32(0, "scale")?,
    });
}

/// Named timers the host counts down; a finished one appears in the
/// `timers_finished` list on the next hook.
fn timers(vm: &VirtualMachine, m: &Py<PyDict>) {
    // start_timer(name, duration, repeat=False)
    command!(vm, m, "start_timer", |a| ScriptCommand::StartTimer {
        name: a.str(0, "name")?,
        duration: a.f32(1, "duration")?,
        repeat: a.opt_bool(2)?.unwrap_or(false),
    });
    command!(vm, m, "stop_timer", |a| ScriptCommand::StopTimer {
        name: a.str(0, "name")?,
    });
}

/// Visibility and the material a mesh draws with.
fn rendering(vm: &VirtualMachine, m: &Py<PyDict>) {
    command!(vm, m, "set_visibility", |a| ScriptCommand::SetVisibility {
        entity_id: None,
        visible: a.bool(0, "visible")?,
    });
    // set_material_color(r, g, b, a=1.0), or set_material_color(color).
    command!(vm, m, "set_material_color", |a| {
        let color = match a.get(0).and_then(|v| crate::convert::shape(v, a.vm())) {
            Some(crate::convert::Shape::Color(c)) => c,
            Some(crate::convert::Shape::Vec3([r, g, b])) => [r, g, b, 1.0],
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
fn cursor_and_camera(vm: &VirtualMachine, m: &Py<PyDict>) {
    command!(vm, m, "lock_cursor", |a| {
        let _ = a;
        ScriptCommand::LockCursor
    });
    command!(vm, m, "unlock_cursor", |a| {
        let _ = a;
        ScriptCommand::UnlockCursor
    });
    command!(vm, m, "screen_shake", |a| ScriptCommand::ScreenShake {
        intensity: a.f32(0, "intensity")?,
        duration: a.f32(1, "duration")?,
    });
}

/// Spawning, despawning and finding entities.
fn ecs(vm: &VirtualMachine, m: &Py<PyDict>) {
    command!(vm, m, "spawn_entity", |a| ScriptCommand::SpawnEntity {
        name: a.str(0, "name")?,
    });
    // spawn_primitive(name, kind, x, y, z, r=None, g=None, b=None)
    //   kind: "cube" | "sphere" | "wall" | ... (any id in ShapeRegistry)
    //   r/g/b: optional; without all three the shape keeps its registered tint.
    // The position may also be one vec3, and the colour one color/vec3.
    command!(vm, m, "spawn_primitive", |a| {
        let name = a.str(0, "name")?;
        let kind = a.str(1, "kind")?;
        let (position, used) = a.xyz_at(2)?;
        let at = 2 + used;
        let color = match a.get(at).and_then(|v| crate::convert::shape(v, a.vm())) {
            Some(crate::convert::Shape::Color(c)) => Some(c),
            Some(crate::convert::Shape::Vec3([r, g, b])) => Some([r, g, b, 1.0]),
            _ => match (
                a.opt_f32(at, "r")?,
                a.opt_f32(at + 1, "g")?,
                a.opt_f32(at + 2, "b")?,
            ) {
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
    command!(vm, m, "despawn_self", |a| {
        let _ = a;
        ScriptCommand::DespawnSelf
    });
    // despawn_by_prefix("chunk_3_5_"): every entity whose Name starts with the
    // prefix, so a streaming script releases a chunk in one call.
    command!(vm, m, "despawn_by_prefix", |a| ScriptCommand::DespawnByPrefix {
        prefix: a.str(0, "prefix")?,
    });
}

/// Loading a different scene, and the sun, sky and fog.
fn scene_and_environment(vm: &VirtualMachine, m: &Py<PyDict>) {
    command!(vm, m, "load_scene", |a| ScriptCommand::LoadScene {
        path: a.str(0, "path")?,
    });
    command!(vm, m, "set_sun_angles", |a| ScriptCommand::SetSunAngles {
        azimuth: a.f32(0, "azimuth")?,
        elevation: a.f32(1, "elevation")?,
    });
    command!(vm, m, "set_fog", |a| ScriptCommand::SetFog {
        enabled: a.bool(0, "enabled")?,
        start: a.f32(1, "start")?,
        end: a.f32(2, "end")?,
    });
}
