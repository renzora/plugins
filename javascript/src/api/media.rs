//! Sound, animation, and the console and debug draw.

use boa_engine::{Context, JsObject, JsValue};
use renzora::{PropertyValue as PropValue, ScriptCommand};

use super::args::Args;
use super::{add, command};
use crate::buffers::push_command;

pub fn register(ctx: &mut Context, m: &JsObject) {
    audio(ctx, m);
    animation(ctx, m);
    debug(ctx, m);
}

/// Sounds, music and the per-entity `AudioPlayer`.
fn audio(ctx: &mut Context, m: &JsObject) {
    // play_sound(path, volume = 1, bus = "Sfx")
    command!(ctx, m, "play_sound", |a, c| ScriptCommand::PlaySound {
        path: a.str(0, "path")?,
        volume: a.opt_f32(1, "volume")?.unwrap_or(1.0),
        looping: false,
        bus: a.opt_str(2, "bus")?.unwrap_or_else(|| "Sfx".into()),
    });
    command!(ctx, m, "play_sound_looping", |a, c| ScriptCommand::PlaySound {
        path: a.str(0, "path")?,
        volume: a.opt_f32(1, "volume")?.unwrap_or(1.0),
        looping: true,
        bus: "Sfx".into(),
    });
    // play_music(path, volume = 1, fade_in = 0)
    command!(ctx, m, "play_music", |a, c| ScriptCommand::PlayMusic {
        path: a.str(0, "path")?,
        volume: a.opt_f32(1, "volume")?.unwrap_or(1.0),
        fade_in: a.opt_f32(2, "fade_in")?.unwrap_or(0.0),
        bus: "Music".into(),
    });
    command!(ctx, m, "stop_music", |a, c| ScriptCommand::StopMusic {
        fade_out: a.opt_f32(0, "fade_out")?.unwrap_or(0.0),
    });
    command!(ctx, m, "stop_all_sounds", |a, c| {
        let _ = a;
        ScriptCommand::StopAllSounds
    });
    // play_audio(entity_name = null): a one-shot from an entity's AudioPlayer
    // (a random clip from its pool, plus jitter). No name means this entity.
    command!(ctx, m, "play_audio", |a, c| ScriptCommand::Action {
        name: "play_audio_player".to_string(),
        target_entity: a.opt_str(0, "entity")?.filter(|s| !s.is_empty()),
        args: Vec::new(),
    });
}

/// Clips, blending and the animation graph.
fn animation(ctx: &mut Context, m: &JsObject) {
    // play_animation(name, looping = true, speed = 1)
    command!(ctx, m, "play_animation", |a, c| ScriptCommand::PlayAnimation {
        entity_id: None,
        name: a.str(0, "name")?,
        looping: a.opt_bool(1).unwrap_or(true),
        speed: a.opt_f32(2, "speed")?.unwrap_or(1.0),
    });
    command!(ctx, m, "stop_animation", |a, c| {
        let _ = a;
        ScriptCommand::StopAnimation { entity_id: None }
    });
    command!(ctx, m, "pause_animation", |a, c| {
        let _ = a;
        ScriptCommand::PauseAnimation { entity_id: None }
    });
    command!(ctx, m, "resume_animation", |a, c| {
        let _ = a;
        ScriptCommand::ResumeAnimation { entity_id: None }
    });
    command!(ctx, m, "set_animation_speed", |a, c| ScriptCommand::SetAnimationSpeed {
        entity_id: None,
        speed: a.f32(0, "speed")?,
    });
    command!(ctx, m, "seek_animation", |a, c| ScriptCommand::SeekAnimation {
        entity_id: None,
        time: a.f32(0, "time")?,
    });
    add(ctx, m, "get_animation_time", |_, _, _| {
        Ok(JsValue::from(
            match renzora::get_handler::call_get(None, "AnimatorReadState", "time") {
                Some(PropValue::Float(f)) => f as f64,
                _ => 0.0,
            },
        ))
    });
    add(ctx, m, "is_animation_playing", |_, _, _| {
        Ok(JsValue::from(matches!(
            renzora::get_handler::call_get(None, "AnimatorReadState", "playing"),
            Some(PropValue::Bool(true))
        )))
    });
    // crossfade_animation(name, duration, looping = true)
    command!(ctx, m, "crossfade_animation", |a, c| ScriptCommand::CrossfadeAnimation {
        entity_id: None,
        name: a.str(0, "name")?,
        duration: a.f32(1, "duration")?,
        looping: a.opt_bool(2).unwrap_or(true),
    });
    command!(ctx, m, "set_anim_param", |a, c| ScriptCommand::SetAnimationParam {
        entity_id: None,
        name: a.str(0, "name")?,
        value: a.f32(1, "value")?,
    });
    command!(ctx, m, "set_anim_bool", |a, c| ScriptCommand::SetAnimationBoolParam {
        entity_id: None,
        name: a.str(0, "name")?,
        value: a.bool(1, "value")?,
    });
    command!(ctx, m, "trigger_anim", |a, c| ScriptCommand::TriggerAnimation {
        entity_id: None,
        name: a.str(0, "name")?,
    });
    command!(ctx, m, "set_layer_weight", |a, c| ScriptCommand::SetAnimationLayerWeight {
        entity_id: None,
        layer_name: a.str(0, "layer")?,
        weight: a.f32(1, "weight")?,
    });
}

/// The console, and lines drawn in the world for a moment.
fn debug(ctx: &mut Context, m: &JsObject) {
    // print_log(message): takes anything, as console.log does.
    command!(ctx, m, "print_log", |a, c| ScriptCommand::Log {
        level: "Info".into(),
        message: a.display(0, c),
    });
    // draw_line(sx, sy, sz, ex, ey, ez, duration = 0) or draw_line(start, end,
    // duration = 0) with two vectors.
    add(ctx, m, "draw_line", |_, args, ctx| {
        let a = Args::new(args, "draw_line");
        let (start, used) = a.xyz_at(0, ctx)?;
        let (end, used_end) = a.xyz_at(used, ctx)?;
        push_command(ScriptCommand::DrawLine {
            start,
            end,
            color: [1.0, 0.0, 0.0, 1.0],
            duration: a.opt_f32(used + used_end, "duration")?.unwrap_or(0.0),
        });
        Ok(JsValue::undefined())
    });
    // The prelude's `console` writes through this, which is how
    // `console.log` reaches the engine console: `_log(level, line)`.
    command!(ctx, m, "_log", |a, c| ScriptCommand::Log {
        level: a.opt_str(0, "level")?.unwrap_or_else(|| "Info".into()),
        message: a.display(1, c),
    });
}
