//! Tools that reach into the game Play is running, rather than the editor:
//! see it, and play it.
//!
//! Everything else in this plugin already sees the running game, because while
//! one is connected the editor's own world mirrors it (`renzora_live_link`):
//! `scene_tree` lists what the game spawned, `inspect_entity` reads the
//! player's live position, `set_component_field` changes the game. What the
//! mirror cannot give is the game's picture and the game's input, and those
//! two travel over the same link through [`renzora::GameLink`].

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use renzora::serde_json::Value;
use renzora::{GameInput, GameLink};

use crate::{b64, ToolReply};

/// Default hold for a press with no `hold_seconds`: long enough that a script
/// polling once a frame sees it, short enough to read as a tap.
const DEFAULT_HOLD: f32 = 0.15;

/// Shorter than the RPC's own screenshot timeout, so the caller hears this
/// module's specific answer rather than a generic one.
const SHOT_DEADLINE: Duration = Duration::from_secs(20);

pub struct PendingGameShot {
    id: u64,
    reply: Sender<ToolReply>,
    deadline: Instant,
}

#[derive(Resource, Default)]
pub struct PendingGameShots(pub Vec<PendingGameShot>);

/// The link, or the reason there is none to use.
fn link(world: &mut World) -> Result<Mut<'_, GameLink>, String> {
    let link = world
        .get_resource_mut::<GameLink>()
        .ok_or("this editor has no live link to a running game")?;
    if !link.connected {
        return Err(
            "no game is running, or it has not connected yet: set_play_mode play, then wait a second or two"
                .into(),
        );
    }
    Ok(link)
}

/// Ask the game for a picture of its window; answered by
/// [`collect_game_screenshots`] when it arrives.
pub fn start_game_screenshot(world: &mut World, reply: Sender<ToolReply>) {
    let id = match link(world) {
        Ok(mut link) => link.request_screenshot(),
        Err(why) => {
            let _ = reply.send(ToolReply::Failed(why));
            return;
        }
    };
    if let Some(mut pending) = world.get_resource_mut::<PendingGameShots>() {
        pending.0.push(PendingGameShot { id, reply, deadline: Instant::now() + SHOT_DEADLINE });
    }
}

pub fn collect_game_screenshots(world: &mut World) {
    if world.get_resource::<PendingGameShots>().is_none_or(|p| p.0.is_empty()) {
        return;
    }
    let mut pending = std::mem::take(&mut world.resource_mut::<PendingGameShots>().0);
    let now = Instant::now();
    pending.retain(|shot| {
        let arrived = world
            .get_resource_mut::<GameLink>()
            .and_then(|mut link| link.screenshots.remove(&shot.id));
        match arrived {
            Some(Ok(png)) => {
                let _ = shot.reply.send(ToolReply::Image { data: b64::encode(&png), mime: "image/png" });
                false
            }
            Some(Err(why)) => {
                let _ = shot.reply.send(ToolReply::Failed(format!("the game could not take the screenshot: {why}")));
                false
            }
            None if now >= shot.deadline => {
                let _ = shot.reply.send(ToolReply::Failed("the game's screenshot never arrived".into()));
                false
            }
            None => true,
        }
    });
    world.resource_mut::<PendingGameShots>().0.extend(pending);
}

/// Press keys and gamepad controls in the running game, each held for its
/// `hold_seconds` and then released.
pub fn game_input(world: &mut World, args: &Value) -> ToolReply {
    let Some(actions) = args.get("actions").and_then(Value::as_array) else {
        return ToolReply::Failed("game_input needs actions, a list".into());
    };
    let mut queued = Vec::new();
    for (i, action) in actions.iter().enumerate() {
        let hold = action
            .get("hold_seconds")
            .and_then(Value::as_f64)
            .map(|h| h as f32)
            .unwrap_or(DEFAULT_HOLD)
            .max(0.0);
        let value = action.get("value").and_then(Value::as_f64).map(|v| v as f32);
        let input = if let Some(code) = action.get("key").and_then(Value::as_str) {
            GameInput::Key { code: code.to_string(), pressed: true }
        } else if let Some(button) = action.get("gamepad_button").and_then(Value::as_str) {
            GameInput::GamepadButton { button: button.to_string(), value: value.unwrap_or(1.0) }
        } else if let Some(axis) = action.get("gamepad_axis").and_then(Value::as_str) {
            let Some(value) = value else {
                return ToolReply::Failed(format!("action {i}: a gamepad_axis needs a value from -1 to 1"));
            };
            GameInput::GamepadAxis { axis: axis.to_string(), value }
        } else {
            return ToolReply::Failed(format!(
                "action {i}: give one of key, gamepad_button or gamepad_axis"
            ));
        };
        queued.push((input, Some(hold)));
    }
    let count = queued.len();
    match link(world) {
        Ok(mut link) => link.inputs.extend(queued),
        Err(why) => return ToolReply::Failed(why),
    }
    ToolReply::Text(format!(
        "sent {count} input(s) to the game; each is released after its hold time. game_screenshot shows the result"
    ))
}
