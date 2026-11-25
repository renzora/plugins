//! Sound, animation, and the console and debug draw.

use renzora::serde_json::Value;
use renzora::{get_handler as host, PropertyValue as PropValue, ScriptCommand};

use super::{push, Args, Out};

pub fn call(name: &str, a: &Args) -> Option<Out> {
    Some(match name {
        // -- Audio --
        // play_sound(path, volume = 1, bus = "Sfx")
        "play_sound" => (|| {
            push(ScriptCommand::PlaySound {
                path: a.str(0, "path")?,
                volume: a.opt_f32(1, "volume")?.unwrap_or(1.0),
                looping: false,
                bus: a.opt_str(2, "bus")?.unwrap_or_else(|| "Sfx".into()),
            })
        })(),
        "play_sound_looping" => (|| {
            push(ScriptCommand::PlaySound {
                path: a.str(0, "path")?,
                volume: a.opt_f32(1, "volume")?.unwrap_or(1.0),
                looping: true,
                bus: "Sfx".into(),
            })
        })(),
        // play_music(path, volume = 1, fade_in = 0)
        "play_music" => (|| {
            push(ScriptCommand::PlayMusic {
                path: a.str(0, "path")?,
                volume: a.opt_f32(1, "volume")?.unwrap_or(1.0),
                fade_in: a.opt_f32(2, "fade_in")?.unwrap_or(0.0),
                bus: "Music".into(),
            })
        })(),
        "stop_music" => (|| {
            push(ScriptCommand::StopMusic {
                fade_out: a.opt_f32(0, "fade_out")?.unwrap_or(0.0),
            })
        })(),
        "stop_all_sounds" => push(ScriptCommand::StopAllSounds),
        // play_audio(entity = null): a one-shot from an entity's AudioPlayer
        // (a random clip from its pool, plus jitter). No name means this one.
        "play_audio" => (|| {
            push(ScriptCommand::Action {
                name: "play_audio_player".to_string(),
                target_entity: a.opt_str(0, "entity")?.filter(|s| !s.is_empty()),
                args: Vec::new(),
            })
        })(),

        // -- Animation --
        // play_animation(name, looping = true, speed = 1)
        "play_animation" => (|| {
            push(ScriptCommand::PlayAnimation {
                entity_id: None,
                name: a.str(0, "name")?,
                looping: a.opt_bool(1, "looping")?.unwrap_or(true),
                speed: a.opt_f32(2, "speed")?.unwrap_or(1.0),
            })
        })(),
        "stop_animation" => push(ScriptCommand::StopAnimation { entity_id: None }),
        "pause_animation" => push(ScriptCommand::PauseAnimation { entity_id: None }),
        "resume_animation" => push(ScriptCommand::ResumeAnimation { entity_id: None }),
        "set_animation_speed" => a
            .f32(0, "speed")
            .and_then(|speed| push(ScriptCommand::SetAnimationSpeed { entity_id: None, speed })),
        "seek_animation" => a
            .f32(0, "time")
            .and_then(|time| push(ScriptCommand::SeekAnimation { entity_id: None, time })),
        "get_animation_time" => Ok(Value::from(match host::call_get(None, "AnimatorReadState", "time") {
            Some(PropValue::Float(f)) => f as f64,
            _ => 0.0,
        })),
        "is_animation_playing" => Ok(Value::Bool(matches!(
            host::call_get(None, "AnimatorReadState", "playing"),
            Some(PropValue::Bool(true))
        ))),
        // crossfade_animation(name, duration, looping = true)
        "crossfade_animation" => (|| {
            push(ScriptCommand::CrossfadeAnimation {
                entity_id: None,
                name: a.str(0, "name")?,
                duration: a.f32(1, "duration")?,
                looping: a.opt_bool(2, "looping")?.unwrap_or(true),
            })
        })(),
        "set_anim_param" => (|| {
            push(ScriptCommand::SetAnimationParam {
                entity_id: None,
                name: a.str(0, "name")?,
                value: a.f32(1, "value")?,
            })
        })(),
        "set_anim_bool" => (|| {
            push(ScriptCommand::SetAnimationBoolParam {
                entity_id: None,
                name: a.str(0, "name")?,
                value: a.bool(1, "value")?,
            })
        })(),
        "trigger_anim" => a
            .str(0, "name")
            .and_then(|name| push(ScriptCommand::TriggerAnimation { entity_id: None, name })),
        "set_layer_weight" => (|| {
            push(ScriptCommand::SetAnimationLayerWeight {
                entity_id: None,
                layer_name: a.str(0, "layer")?,
                weight: a.f32(1, "weight")?,
            })
        })(),

        // -- The console and debug draw --
        // print_log(message): takes anything.
        "print_log" => push(ScriptCommand::Log {
            level: "Info".into(),
            message: a.display(0),
        }),
        // draw_line(start, end, duration = 0), or with six numbers.
        "draw_line" => (|| {
            let (start, used) = a.xyz_at(0)?;
            let (end, used_end) = a.xyz_at(used)?;
            push(ScriptCommand::DrawLine {
                start,
                end,
                color: a.opt_color(used + used_end + 1).unwrap_or([1.0, 0.0, 0.0, 1.0]),
                duration: a.opt_f32(used + used_end, "duration")?.unwrap_or(0.0),
            })
        })(),
        _ => return None,
    })
}
