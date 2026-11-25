//! Sound, animation, and the console and debug draw.

use renzora::{PropertyValue as PropValue, ScriptCommand};
use rustpython_vm::builtins::PyDict;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::{Py, PyResult, VirtualMachine};

use super::args::Args;
use super::{add, command};
use crate::buffers::push_command;

pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    audio(vm, m);
    animation(vm, m);
    debug(vm, m);
}

/// Sounds, music and the per-entity `AudioPlayer`.
fn audio(vm: &VirtualMachine, m: &Py<PyDict>) {
    // play_sound(path, volume=1.0, bus="Sfx")
    command!(vm, m, "play_sound", |a| ScriptCommand::PlaySound {
        path: a.str(0, "path")?,
        volume: a.opt_f32(1, "volume")?.unwrap_or(1.0),
        looping: false,
        bus: a.opt_str(2, "bus")?.unwrap_or_else(|| "Sfx".into()),
    });
    command!(vm, m, "play_sound_looping", |a| ScriptCommand::PlaySound {
        path: a.str(0, "path")?,
        volume: a.opt_f32(1, "volume")?.unwrap_or(1.0),
        looping: true,
        bus: "Sfx".into(),
    });
    // play_music(path, volume=1.0, fade_in=0.0)
    command!(vm, m, "play_music", |a| ScriptCommand::PlayMusic {
        path: a.str(0, "path")?,
        volume: a.opt_f32(1, "volume")?.unwrap_or(1.0),
        fade_in: a.opt_f32(2, "fade_in")?.unwrap_or(0.0),
        bus: "Music".into(),
    });
    command!(vm, m, "stop_music", |a| ScriptCommand::StopMusic {
        fade_out: a.opt_f32(0, "fade_out")?.unwrap_or(0.0),
    });
    command!(vm, m, "stop_all_sounds", |a| {
        let _ = a;
        ScriptCommand::StopAllSounds
    });
    // play_audio(entity_name=None): a one-shot from an entity's AudioPlayer
    // (a random clip from its pool, plus jitter). No name means this entity.
    command!(vm, m, "play_audio", |a| ScriptCommand::Action {
        name: "play_audio_player".to_string(),
        target_entity: a.opt_str(0, "entity")?.filter(|s| !s.is_empty()),
        args: Vec::new(),
    });
}

/// Clips, blending and the animation graph.
fn animation(vm: &VirtualMachine, m: &Py<PyDict>) {
    // play_animation(name, looping=True, speed=1.0)
    command!(vm, m, "play_animation", |a| ScriptCommand::PlayAnimation {
        entity_id: None,
        name: a.str(0, "name")?,
        looping: a.opt_bool(1)?.unwrap_or(true),
        speed: a.opt_f32(2, "speed")?.unwrap_or(1.0),
    });
    command!(vm, m, "stop_animation", |a| {
        let _ = a;
        ScriptCommand::StopAnimation { entity_id: None }
    });
    command!(vm, m, "pause_animation", |a| {
        let _ = a;
        ScriptCommand::PauseAnimation { entity_id: None }
    });
    command!(vm, m, "resume_animation", |a| {
        let _ = a;
        ScriptCommand::ResumeAnimation { entity_id: None }
    });
    command!(vm, m, "set_animation_speed", |a| ScriptCommand::SetAnimationSpeed {
        entity_id: None,
        speed: a.f32(0, "speed")?,
    });
    command!(vm, m, "seek_animation", |a| ScriptCommand::SeekAnimation {
        entity_id: None,
        time: a.f32(0, "time")?,
    });
    add(vm, m, "get_animation_time", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<f64> {
        Ok(match renzora::get_handler::call_get(None, "AnimatorReadState", "time") {
            Some(PropValue::Float(f)) => f as f64,
            _ => 0.0,
        })
    });
    add(vm, m, "is_animation_playing", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<bool> {
        Ok(matches!(
            renzora::get_handler::call_get(None, "AnimatorReadState", "playing"),
            Some(PropValue::Bool(true))
        ))
    });
    // crossfade_animation(name, duration, looping=True)
    command!(vm, m, "crossfade_animation", |a| ScriptCommand::CrossfadeAnimation {
        entity_id: None,
        name: a.str(0, "name")?,
        duration: a.f32(1, "duration")?,
        looping: a.opt_bool(2)?.unwrap_or(true),
    });
    command!(vm, m, "set_anim_param", |a| ScriptCommand::SetAnimationParam {
        entity_id: None,
        name: a.str(0, "name")?,
        value: a.f32(1, "value")?,
    });
    command!(vm, m, "set_anim_bool", |a| ScriptCommand::SetAnimationBoolParam {
        entity_id: None,
        name: a.str(0, "name")?,
        value: a.bool(1, "value")?,
    });
    command!(vm, m, "trigger_anim", |a| ScriptCommand::TriggerAnimation {
        entity_id: None,
        name: a.str(0, "name")?,
    });
    command!(vm, m, "set_layer_weight", |a| ScriptCommand::SetAnimationLayerWeight {
        entity_id: None,
        layer_name: a.str(0, "layer")?,
        weight: a.f32(1, "weight")?,
    });
}

/// The console, and lines drawn in the world for a moment.
fn debug(vm: &VirtualMachine, m: &Py<PyDict>) {
    // print_log(message): takes anything, as print() does.
    command!(vm, m, "print_log", |a| ScriptCommand::Log {
        level: "Info".into(),
        message: a.display(0),
    });
    // draw_line(sx, sy, sz, ex, ey, ez, duration=0) or draw_line(start, end,
    // duration=0) with two vec3s.
    add(vm, m, "draw_line", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "draw_line");
        let (start, used) = a.xyz_at(0)?;
        let (end, used_end) = a.xyz_at(used)?;
        push_command(ScriptCommand::DrawLine {
            start,
            end,
            color: [1.0, 0.0, 0.0, 1.0],
            duration: a.opt_f32(used + used_end, "duration")?.unwrap_or(0.0),
        });
        Ok(())
    });
    // The prelude's stdout/stderr write through this, which is how `print`
    // reaches the engine console: `_log(level, line)`.
    command!(vm, m, "_log", |a| ScriptCommand::Log {
        level: a.opt_str(0, "level")?.unwrap_or_else(|| "Info".into()),
        message: a.display(1),
    });
}
