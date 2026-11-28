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

/// The IME caret for a preedit: after the text by default, `null` to hide
/// it, or `[start, end]` byte offsets, which must fall on character
/// boundaries because a real IME's always do.
fn ime_cursor(value: &str, cursor: Option<&Value>) -> Result<Option<(usize, usize)>, String> {
    match cursor {
        None => Ok(Some((value.len(), value.len()))),
        Some(Value::Null) => Ok(None),
        Some(Value::Array(ab)) => {
            let n = |i: usize| ab.get(i).and_then(Value::as_u64).map(|v| v as usize);
            let (Some(a), Some(b), 2) = (n(0), n(1), ab.len()) else {
                return Err("cursor is [start, end] in bytes of the preedit, or null".into());
            };
            if !value.is_char_boundary(a) || !value.is_char_boundary(b) {
                return Err(format!("cursor [{a}, {b}] does not fall on character boundaries of \"{value}\""));
            }
            Ok(Some((a, b)))
        }
        Some(_) => Err("cursor is [start, end] in bytes of the preedit, or null".into()),
    }
}

/// Press keys and gamepad controls, move and click the mouse, type, and send
/// IME composition in the running game. Buttons and keys are held for their
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
        let consumed = action.get("consumed").and_then(Value::as_bool).unwrap_or(true);
        // Done the moment they are sent: nothing to release.
        let instant = if let Some(xy) = action.get("pointer").and_then(Value::as_array) {
            let n = |i: usize| xy.get(i).and_then(Value::as_f64).map(|v| v as f32);
            let (Some(x), Some(y)) = (n(0), n(1)) else {
                return ToolReply::Failed(format!("action {i}: pointer is [x, y] in game_screenshot pixels"));
            };
            Some(GameInput::Pointer { x, y })
        } else if let Some(text) = action.get("text").and_then(Value::as_str) {
            Some(GameInput::Text { text: text.to_string() })
        } else if let Some(preedit) = action.get("ime_preedit").and_then(Value::as_str) {
            let cursor = match ime_cursor(preedit, action.get("cursor")) {
                Ok(c) => c,
                Err(why) => return ToolReply::Failed(format!("action {i}: {why}")),
            };
            Some(GameInput::ImePreedit { value: preedit.to_string(), cursor, consumed })
        } else if let Some(commit) = action.get("ime_commit").and_then(Value::as_str) {
            Some(GameInput::ImeCommit { value: commit.to_string(), consumed })
        } else {
            None
        };
        if let Some(input) = instant {
            queued.push((input, None));
            continue;
        }
        let input = if let Some(button) = action.get("mouse_button").and_then(Value::as_str) {
            GameInput::MouseButton { button: button.to_string(), pressed: true }
        } else if let Some(code) = action.get("key").and_then(Value::as_str) {
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
                "action {i}: give one of key, gamepad_button, gamepad_axis, pointer, mouse_button, text, ime_preedit or ime_commit"
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
