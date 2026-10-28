//! Driving the editor's interface the way a person does: a pointer that moves,
//! presses and drags, a keyboard that types, and a way to find what to aim at.
//!
//! The other tools each reach one feature through a seam made for it, and a
//! catalogue built that way always trails the editor: every new panel, mode and
//! button would need its own tool before an agent could touch it. Input has no
//! such gap. A click reaches every button there is, including ones from plugins
//! this server has never heard of, and it goes through exactly the code a
//! person's click does, so whatever works for the agent works for the user too.
//!
//! # Injection
//!
//! Events are written as the messages winit would have written: the specific
//! message (`CursorMoved`, `MouseButtonInput`, `KeyboardInput`, `MouseWheel`)
//! for `ButtonInput` and the systems that read those, and its `WindowEvent`
//! copy for picking, which reads only that. They go in from `First`, ahead of
//! picking's input pass (which runs there) and the `PreUpdate` input set, so
//! both see the same frame's events, as they do for a real click.
//!
//! One step per frame. A press and its release in the same frame is a click
//! some widgets never see, because ember clears a press in `Last` only once the
//! button is up, and hover has to land before a press for the right node to take
//! it. Typing is the exception: text fields read every key message in a frame,
//! in order, so a character's press and release can share one.
//!
//! # The user's own cursor is left alone
//!
//! Plenty of editor code reads `Window::cursor_position` rather than picking, so
//! the window's cursor is moved to the agent's pointer for the frame. It is put
//! back in `PostUpdate`. Left moved, bevy_winit would see it differ from its
//! cache in `Last` and warp the operating system's cursor there, taking the
//! mouse out from under the person watching.
//!
//! # The MCP cursor
//!
//! What the agent's pointer is doing is drawn over the editor as a cursor of its
//! own, in a colour no editor chrome uses, with a pulse on every press and a
//! caption naming the action. Without it an agent's click is a button that
//! changes state by itself, which is hard to follow and impossible to tell from
//! a bug.

use bevy::ecs::message::MessageUpdateSystems;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKeyCode};
use bevy::input::mouse::{MouseButtonInput, MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::input::touch::TouchPhase;
use bevy::input::{ButtonState, InputSystems};
use bevy::math::DVec2;
use bevy::picking::PickingSystems;
use bevy::prelude::*;
use bevy::ui::{CalculatedClip, ComputedNode, Pressed, UiGlobalTransform, UiScale};
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};
use renzora::serde_json::{json, Value};
use renzora::HideInHierarchy;
use renzora_ember::workspace::WorkspaceSwitch;
use renzora_ember::UiInteraction;
use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::tools::entity_id;
use crate::ToolReply;

/// The MCP cursor's colour. Warm, because the editor's own chrome is cool, so the
/// agent's pointer never reads as part of the interface it is pointing at.
const INK: (u8, u8, u8) = (232, 124, 72);

/// How long the cursor stays on screen after the last action, so a person who
/// looked away for a moment still sees where it ended up.
const LINGER: Duration = Duration::from_millis(2500);

/// How long an input run may take before it is abandoned and every held button
/// and key let go. Shorter than the caller's timeout, so the caller hears why.
const JOB_LIMIT: Duration = Duration::from_secs(50);

/// Characters typed per frame. Two keeps typing visible as typing while a
/// sentence still finishes in a second or two at editor frame rates.
const CHARS_PER_FRAME: usize = 2;

/// The tools that act through input and therefore answer a few frames later.
pub fn is_input_tool(name: &str) -> bool {
    matches!(name, "click" | "drag" | "move_mouse" | "scroll" | "press_keys" | "type_text")
}

// ============================================================================
// The player
// ============================================================================

enum Step {
    /// Move the pointer to a point, in window pixels, over this many frames.
    Glide { to: Vec2, frames: u32 },
    Button { button: MouseButton, down: bool },
    Key { code: KeyCode, key: Key, down: bool },
    /// Characters typed as press-and-release pairs, a few per frame.
    Type(VecDeque<char>),
    Wheel { lines: f32 },
    Wait(u32),
}

struct Job {
    steps: VecDeque<Step>,
    reply: Sender<ToolReply>,
    summary: String,
    caption: String,
    /// What took each press, read the frame after it went down.
    took: Vec<String>,
    look_for_press: bool,
    /// Progress through the current glide: where it started and frames done.
    glide: Option<(Vec2, u32)>,
    started: Instant,
}

/// Queued input runs, the agent's pointer, and what it is holding down.
#[derive(Resource, Default)]
pub struct InputPlayer {
    jobs: VecDeque<Job>,
    /// The agent's pointer in window pixels (the screenshot's pixels).
    pos: Option<Vec2>,
    held_buttons: Vec<MouseButton>,
    held_keys: Vec<(KeyCode, Key)>,
    /// The window's own cursor position from before this frame's override, to
    /// put back in `PostUpdate`. Outer `Some` means there is something to undo.
    restore: Option<Option<DVec2>>,
    last_active: Option<Instant>,
    pressing: bool,
    /// When the last press went down, for the cursor's pulse.
    pulse: Option<Instant>,
    caption: String,
}

pub fn build(app: &mut App) {
    app.init_resource::<InputPlayer>()
        // In `First`, between the message swap and picking's input pass, which
        // runs there rather than in `PreUpdate`. Written any later, picking read
        // each press a frame after `ButtonInput` did, and code that needs both
        // on the same frame (the workspace ribbon) never saw a click at all.
        .add_systems(
            First,
            play.after(MessageUpdateSystems)
                .before(PickingSystems::Input)
                .before(InputSystems),
        )
        .add_systems(PostUpdate, restore_cursor)
        .add_systems(Update, draw_cursor);
}

/// Queue an input tool's run. The reply is sent when the run finishes.
pub fn start(world: &mut World, name: &str, args: &Value, reply: Sender<ToolReply>) {
    match plan(world, name, args) {
        Ok((steps, summary, caption)) => {
            world.resource_mut::<InputPlayer>().jobs.push_back(Job {
                steps,
                reply,
                summary,
                caption,
                took: Vec::new(),
                look_for_press: false,
                glide: None,
                started: Instant::now(),
            });
        }
        Err(why) => {
            let _ = reply.send(ToolReply::Failed(why));
        }
    }
}

/// Run one frame of the front job.
fn play(world: &mut World) {
    let Some(window) = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .iter(world)
        .next()
    else {
        return;
    };

    let mut player = std::mem::take(&mut *world.resource_mut::<InputPlayer>());
    if player.jobs.is_empty() {
        *world.resource_mut::<InputPlayer>() = player;
        return;
    }
    player.last_active = Some(Instant::now());

    // Read before this frame's input: the press that went down last frame has
    // been resolved by picking, and its release has not happened yet.
    if player.jobs.front().is_some_and(|j| j.look_for_press) {
        let names = pressed_now(world);
        if let Some(job) = player.jobs.front_mut() {
            job.look_for_press = false;
            job.took.extend(names);
        }
    }

    if player.jobs.front().is_some_and(|j| j.started.elapsed() > JOB_LIMIT) {
        let job = player.jobs.pop_front();
        release_everything(world, window, &mut player);
        if let Some(job) = job {
            let _ = job.reply.send(ToolReply::Failed(format!(
                "{} did not finish in {}s; every held button and key was released",
                job.summary,
                JOB_LIMIT.as_secs()
            )));
        }
        *world.resource_mut::<InputPlayer>() = player;
        return;
    }

    // Hold the window's cursor on the agent's pointer for this frame, so code
    // reading it rather than picking agrees with where the pointer is.
    if let Some(pos) = player.pos {
        place_window_cursor(world, window, pos, &mut player);
    }

    let caption = player.jobs.front().map(|j| j.caption.clone()).unwrap_or_default();
    player.caption = caption;

    let finished = step(world, window, &mut player);
    if finished {
        if let Some(job) = player.jobs.pop_front() {
            let mut text = job.summary.clone();
            let mut took = job.took.clone();
            took.dedup();
            if !took.is_empty() {
                text.push_str(&format!(". The press went to: {}", took.join("; ")));
            }
            let _ = job.reply.send(ToolReply::Text(text));
        }
    }
    *world.resource_mut::<InputPlayer>() = player;
}

/// Advance the front job by one frame. True once it has nothing left to do.
fn step(world: &mut World, window: Entity, player: &mut InputPlayer) -> bool {
    let Some(job) = player.jobs.front_mut() else {
        return true;
    };
    let Some(current) = job.steps.front_mut() else {
        return true;
    };

    let mut done = true;
    match current {
        Step::Glide { to, frames } => {
            let (from, n) = job.glide.unwrap_or((player.pos.unwrap_or(*to), 0));
            let total = (*frames).max(1);
            let n = n + 1;
            // Ease out, so the pointer arrives the way a hand does and the last
            // frames, where hover changes, are the slow ones.
            let t = n as f32 / total as f32;
            let eased = 1.0 - (1.0 - t).powi(3);
            let at = from.lerp(*to, eased);
            let previous = player.pos.unwrap_or(at);
            player.pos = Some(at);
            write_move(world, window, previous, at);
            if n < total {
                job.glide = Some((from, n));
                done = false;
            } else {
                job.glide = None;
            }
        }
        Step::Button { button, down } => {
            let (button, down) = (*button, *down);
            write_button(world, window, button, down);
            if down {
                player.held_buttons.push(button);
                player.pressing = true;
                player.pulse = Some(Instant::now());
                job.look_for_press = true;
            } else {
                player.held_buttons.retain(|b| *b != button);
                player.pressing = !player.held_buttons.is_empty();
            }
        }
        Step::Key { code, key, down } => {
            let (code, key, down) = (*code, key.clone(), *down);
            write_key(world, window, code, key.clone(), down, None);
            if down {
                player.held_keys.push((code, key));
            } else {
                player.held_keys.retain(|(c, _)| *c != code);
            }
        }
        Step::Type(chars) => {
            for _ in 0..CHARS_PER_FRAME {
                let Some(ch) = chars.pop_front() else { break };
                let (code, key) = char_key(ch);
                let text = matches!(key, Key::Character(_) | Key::Space).then(|| ch.to_string());
                write_key(world, window, code, key.clone(), true, text);
                write_key(world, window, code, key, false, None);
            }
            done = chars.is_empty();
        }
        Step::Wheel { lines } => {
            let lines = *lines;
            write_wheel(world, window, lines);
        }
        Step::Wait(frames) => {
            if *frames > 1 {
                *frames -= 1;
                done = false;
            }
        }
    }
    if done {
        job.steps.pop_front();
    }
    false
}

/// Let go of everything an abandoned run was holding, so a button the agent
/// pressed is never left down for the person to discover.
fn release_everything(world: &mut World, window: Entity, player: &mut InputPlayer) {
    for button in std::mem::take(&mut player.held_buttons) {
        write_button(world, window, button, false);
    }
    for (code, key) in std::mem::take(&mut player.held_keys) {
        write_key(world, window, code, key, false, None);
    }
    player.pressing = false;
}

/// Labels of the UI nodes holding a press right now.
fn pressed_now(world: &mut World) -> Vec<String> {
    let hits: Vec<Entity> = world
        .query_filtered::<Entity, (With<Pressed>, With<UiInteraction>)>()
        .iter(world)
        .collect();
    hits.into_iter().map(|e| describe(world, e)).collect()
}

fn place_window_cursor(world: &mut World, window: Entity, pos: Vec2, player: &mut InputPlayer) {
    if let Some(mut win) = world.get_mut::<Window>(window) {
        if player.restore.is_none() {
            player.restore = Some(win.physical_cursor_position().map(|p| p.as_dvec2()));
        }
        win.set_physical_cursor_position(Some(pos.as_dvec2()));
    }
}

/// Put the window's cursor back where the person's mouse is. See the module
/// docs: left on the agent's pointer, it would warp the real cursor.
fn restore_cursor(mut player: ResMut<InputPlayer>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let Some(saved) = player.restore.take() else {
        return;
    };
    if let Ok(mut window) = windows.single_mut() {
        window.set_physical_cursor_position(saved);
    }
}

fn scale_factor(world: &World, window: Entity) -> f32 {
    world.get::<Window>(window).map(|w| w.scale_factor()).unwrap_or(1.0)
}

fn write_move(world: &mut World, window: Entity, from: Vec2, to: Vec2) {
    let scale = scale_factor(world, window);
    let position = to / scale;
    let delta = (to - from) / scale;
    if let Some(mut win) = world.get_mut::<Window>(window) {
        win.set_physical_cursor_position(Some(to.as_dvec2()));
    }
    let moved = CursorMoved { window, position, delta: Some(delta) };
    world.write_message(moved.clone());
    world.write_message(WindowEvent::from(moved));
    // Orbit and pan read raw motion, which a cursor that is only told where to
    // be never produces.
    if delta != Vec2::ZERO {
        world.write_message(MouseMotion { delta });
    }
}

fn write_button(world: &mut World, window: Entity, button: MouseButton, down: bool) {
    let input = MouseButtonInput {
        button,
        state: if down { ButtonState::Pressed } else { ButtonState::Released },
        window,
    };
    world.write_message(input);
    world.write_message(WindowEvent::from(input));
}

fn write_key(world: &mut World, window: Entity, code: KeyCode, key: Key, down: bool, text: Option<String>) {
    let input = KeyboardInput {
        key_code: code,
        logical_key: key,
        state: if down { ButtonState::Pressed } else { ButtonState::Released },
        text: text.map(Into::into),
        repeat: false,
        window,
    };
    world.write_message(input.clone());
    world.write_message(WindowEvent::from(input));
}

fn write_wheel(world: &mut World, window: Entity, lines: f32) {
    let wheel = MouseWheel { unit: MouseScrollUnit::Line, x: 0.0, y: lines, window, phase: TouchPhase::Moved };
    world.write_message(wheel);
    world.write_message(WindowEvent::from(wheel));
}

// ============================================================================
// Planning: arguments into steps
// ============================================================================

type Plan = (VecDeque<Step>, String, String);

fn plan(world: &mut World, name: &str, args: &Value) -> Result<Plan, String> {
    let mut steps = VecDeque::new();
    let modifiers = parse_modifiers(args)?;
    let button = parse_button(args)?;
    let from = world.resource::<InputPlayer>().pos;

    match name {
        "click" => {
            let (at, what) = resolve_target(world, args.get("target"), args.get("offset"))?;
            let count = args.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, 3);
            steps.push_back(glide(from, at));
            steps.push_back(Step::Wait(1));
            press_modifiers(&mut steps, &modifiers, true);
            for _ in 0..count {
                steps.push_back(Step::Button { button, down: true });
                steps.push_back(Step::Button { button, down: false });
            }
            press_modifiers(&mut steps, &modifiers, false);
            steps.push_back(Step::Wait(2));
            let verb = match (count, button) {
                (2, _) => "double-clicked",
                (3, _) => "triple-clicked",
                (_, MouseButton::Right) => "right-clicked",
                (_, MouseButton::Middle) => "middle-clicked",
                _ => "clicked",
            };
            let summary = format!("{verb}{} {what} at {}", chord_suffix(&modifiers), point(at));
            let caption = format!("{verb} {}", short(&what));
            Ok((steps, summary, caption))
        }
        "drag" => {
            let (a, from_what) = resolve_target(world, args.get("from"), args.get("from_offset"))?;
            let (b, to_what) = resolve_target(world, args.get("to"), args.get("to_offset"))?;
            let frames = args
                .get("frames")
                .and_then(Value::as_u64)
                .map(|f| f.clamp(2, 240) as u32)
                .unwrap_or_else(|| glide_frames(a, b).max(10));
            steps.push_back(glide(from, a));
            steps.push_back(Step::Wait(1));
            press_modifiers(&mut steps, &modifiers, true);
            steps.push_back(Step::Button { button, down: true });
            steps.push_back(Step::Wait(1));
            // A small first nudge, so widgets that start a drag only past a few
            // pixels of travel have started before the long move.
            let nudge = a + (b - a).normalize_or_zero() * 6.0;
            steps.push_back(Step::Glide { to: nudge, frames: 2 });
            steps.push_back(Step::Glide { to: b, frames });
            steps.push_back(Step::Wait(1));
            steps.push_back(Step::Button { button, down: false });
            press_modifiers(&mut steps, &modifiers, false);
            steps.push_back(Step::Wait(2));
            let summary = format!(
                "dragged{} from {from_what} at {} to {to_what} at {}",
                chord_suffix(&modifiers),
                point(a),
                point(b)
            );
            Ok((steps, summary, format!("drag to {}", short(&to_what))))
        }
        "move_mouse" => {
            let (at, what) = resolve_target(world, args.get("target"), args.get("offset"))?;
            steps.push_back(glide(from, at));
            steps.push_back(Step::Wait(3));
            Ok((steps, format!("moved the pointer over {what} at {}", point(at)), format!("hover {}", short(&what))))
        }
        "scroll" => {
            let (at, what) = resolve_target(world, args.get("target"), args.get("offset"))?;
            let lines = args.get("lines").and_then(Value::as_f64).unwrap_or(-3.0) as f32;
            steps.push_back(glide(from, at));
            steps.push_back(Step::Wait(1));
            press_modifiers(&mut steps, &modifiers, true);
            // One notch a frame, as a wheel delivers them, rather than one huge
            // delta some scroll views clamp.
            let notches = lines.abs().ceil().max(1.0) as u32;
            let each = lines / notches as f32;
            for _ in 0..notches.min(60) {
                steps.push_back(Step::Wheel { lines: each });
            }
            press_modifiers(&mut steps, &modifiers, false);
            steps.push_back(Step::Wait(2));
            Ok((steps, format!("scrolled {lines} lines over {what} at {}", point(at)), format!("scroll {}", short(&what))))
        }
        "press_keys" => {
            let chords: Vec<String> = match args.get("keys") {
                Some(Value::String(s)) => vec![s.clone()],
                Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                _ => return Err("press_keys needs keys, such as \"ctrl+s\" or [\"ctrl+a\", \"delete\"]".into()),
            };
            if chords.is_empty() {
                return Err("press_keys needs at least one key".into());
            }
            for chord in &chords {
                let parts = parse_chord(chord)?;
                for (code, key) in &parts {
                    steps.push_back(Step::Key { code: *code, key: key.clone(), down: true });
                }
                for (code, key) in parts.iter().rev() {
                    steps.push_back(Step::Key { code: *code, key: key.clone(), down: false });
                }
                steps.push_back(Step::Wait(1));
            }
            steps.push_back(Step::Wait(1));
            let list = chords.join(", ");
            Ok((steps, format!("pressed {list}"), format!("keys {list}")))
        }
        "type_text" => {
            let Some(text) = args.get("text").and_then(Value::as_str) else {
                return Err("type_text needs text".into());
            };
            if text.chars().count() > 2000 {
                return Err("type_text takes at most 2000 characters at a time".into());
            }
            steps.push_back(Step::Type(text.chars().collect()));
            if args.get("enter").and_then(Value::as_bool).unwrap_or(false) {
                steps.push_back(Step::Key { code: KeyCode::Enter, key: Key::Enter, down: true });
                steps.push_back(Step::Key { code: KeyCode::Enter, key: Key::Enter, down: false });
            }
            steps.push_back(Step::Wait(2));
            Ok((steps, format!("typed {} characters", text.chars().count()), format!("type \"{}\"", short(text))))
        }
        other => Err(format!("{other} is not an input tool")),
    }
}

fn glide(from: Option<Vec2>, to: Vec2) -> Step {
    Step::Glide { to, frames: from.map(|f| glide_frames(f, to)).unwrap_or(1) }
}

/// Frames for a move: long enough to watch, short enough not to drag on.
fn glide_frames(from: Vec2, to: Vec2) -> u32 {
    ((from.distance(to) / 45.0).round() as u32).clamp(3, 12)
}

fn press_modifiers(steps: &mut VecDeque<Step>, modifiers: &[(KeyCode, Key)], down: bool) {
    let list: Vec<&(KeyCode, Key)> = if down { modifiers.iter().collect() } else { modifiers.iter().rev().collect() };
    for (code, key) in list {
        steps.push_back(Step::Key { code: *code, key: key.clone(), down });
    }
}

fn chord_suffix(modifiers: &[(KeyCode, Key)]) -> String {
    if modifiers.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = modifiers
        .iter()
        .map(|(code, _)| match code {
            KeyCode::ControlLeft => "ctrl",
            KeyCode::ShiftLeft => "shift",
            KeyCode::AltLeft => "alt",
            _ => "super",
        })
        .collect();
    format!(" with {}", names.join("+"))
}

fn point(p: Vec2) -> String {
    format!("({:.0}, {:.0})", p.x, p.y)
}

fn short(text: &str) -> String {
    let mut out: String = text.chars().take(32).collect();
    if text.chars().count() > 32 {
        out.push('…');
    }
    out
}

fn parse_button(args: &Value) -> Result<MouseButton, String> {
    match args.get("button").and_then(Value::as_str).unwrap_or("left") {
        "left" => Ok(MouseButton::Left),
        "right" => Ok(MouseButton::Right),
        "middle" => Ok(MouseButton::Middle),
        other => Err(format!("unknown button {other}: use left, right or middle")),
    }
}

fn parse_modifiers(args: &Value) -> Result<Vec<(KeyCode, Key)>, String> {
    let Some(list) = args.get("modifiers").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    list.iter()
        .map(|v| {
            let name = v.as_str().unwrap_or_default();
            modifier(name).ok_or_else(|| format!("unknown modifier {name}: use ctrl, shift, alt or super"))
        })
        .collect()
}

fn modifier(name: &str) -> Option<(KeyCode, Key)> {
    match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some((KeyCode::ControlLeft, Key::Control)),
        "shift" => Some((KeyCode::ShiftLeft, Key::Shift)),
        "alt" | "option" => Some((KeyCode::AltLeft, Key::Alt)),
        "super" | "win" | "meta" | "cmd" => Some((KeyCode::SuperLeft, Key::Super)),
        _ => None,
    }
}

/// `"ctrl+shift+s"` into its keys, modifiers first.
fn parse_chord(chord: &str) -> Result<Vec<(KeyCode, Key)>, String> {
    let parts: Vec<&str> = chord.split('+').map(str::trim).collect();
    let mut out = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        // A literal plus, as in "ctrl++", leaves an empty part.
        let part = if part.is_empty() { "+" } else { part };
        if i + 1 < parts.len() {
            if let Some(m) = modifier(part) {
                out.push(m);
                continue;
            }
        }
        out.push(named_key(part).ok_or_else(|| format!("unknown key {part} in {chord}"))?);
    }
    Ok(out)
}

fn named_key(name: &str) -> Option<(KeyCode, Key)> {
    let lower = name.to_ascii_lowercase();
    let named = match lower.as_str() {
        "enter" | "return" => (KeyCode::Enter, Key::Enter),
        "escape" | "esc" => (KeyCode::Escape, Key::Escape),
        "tab" => (KeyCode::Tab, Key::Tab),
        "space" => (KeyCode::Space, Key::Space),
        "backspace" => (KeyCode::Backspace, Key::Backspace),
        "delete" | "del" => (KeyCode::Delete, Key::Delete),
        "insert" => (KeyCode::Insert, Key::Insert),
        "home" => (KeyCode::Home, Key::Home),
        "end" => (KeyCode::End, Key::End),
        "pageup" => (KeyCode::PageUp, Key::PageUp),
        "pagedown" => (KeyCode::PageDown, Key::PageDown),
        "up" => (KeyCode::ArrowUp, Key::ArrowUp),
        "down" => (KeyCode::ArrowDown, Key::ArrowDown),
        "left" => (KeyCode::ArrowLeft, Key::ArrowLeft),
        "right" => (KeyCode::ArrowRight, Key::ArrowRight),
        "ctrl" | "control" | "shift" | "alt" | "super" | "win" | "meta" | "cmd" | "option" => return modifier(&lower),
        _ => {
            if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                let code = match n {
                    1 => KeyCode::F1,
                    2 => KeyCode::F2,
                    3 => KeyCode::F3,
                    4 => KeyCode::F4,
                    5 => KeyCode::F5,
                    6 => KeyCode::F6,
                    7 => KeyCode::F7,
                    8 => KeyCode::F8,
                    9 => KeyCode::F9,
                    10 => KeyCode::F10,
                    11 => KeyCode::F11,
                    12 => KeyCode::F12,
                    _ => return None,
                };
                let key = match n {
                    1 => Key::F1,
                    2 => Key::F2,
                    3 => Key::F3,
                    4 => Key::F4,
                    5 => Key::F5,
                    6 => Key::F6,
                    7 => Key::F7,
                    8 => Key::F8,
                    9 => Key::F9,
                    10 => Key::F10,
                    11 => Key::F11,
                    _ => Key::F12,
                };
                return Some((code, key));
            }
            let mut chars = lower.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else {
                return None;
            };
            return Some(char_key(ch));
        }
    };
    Some(named)
}

/// The physical key and logical key a character comes from. Text fields read
/// the logical key; shortcuts read the physical one, so both are filled.
fn char_key(ch: char) -> (KeyCode, Key) {
    let code = match ch.to_ascii_lowercase() {
        'a' => KeyCode::KeyA,
        'b' => KeyCode::KeyB,
        'c' => KeyCode::KeyC,
        'd' => KeyCode::KeyD,
        'e' => KeyCode::KeyE,
        'f' => KeyCode::KeyF,
        'g' => KeyCode::KeyG,
        'h' => KeyCode::KeyH,
        'i' => KeyCode::KeyI,
        'j' => KeyCode::KeyJ,
        'k' => KeyCode::KeyK,
        'l' => KeyCode::KeyL,
        'm' => KeyCode::KeyM,
        'n' => KeyCode::KeyN,
        'o' => KeyCode::KeyO,
        'p' => KeyCode::KeyP,
        'q' => KeyCode::KeyQ,
        'r' => KeyCode::KeyR,
        's' => KeyCode::KeyS,
        't' => KeyCode::KeyT,
        'u' => KeyCode::KeyU,
        'v' => KeyCode::KeyV,
        'w' => KeyCode::KeyW,
        'x' => KeyCode::KeyX,
        'y' => KeyCode::KeyY,
        'z' => KeyCode::KeyZ,
        '0' | ')' => KeyCode::Digit0,
        '1' | '!' => KeyCode::Digit1,
        '2' | '@' => KeyCode::Digit2,
        '3' | '#' => KeyCode::Digit3,
        '4' | '$' => KeyCode::Digit4,
        '5' | '%' => KeyCode::Digit5,
        '6' | '^' => KeyCode::Digit6,
        '7' | '&' => KeyCode::Digit7,
        '8' | '*' => KeyCode::Digit8,
        '9' | '(' => KeyCode::Digit9,
        '-' | '_' => KeyCode::Minus,
        '=' | '+' => KeyCode::Equal,
        '[' | '{' => KeyCode::BracketLeft,
        ']' | '}' => KeyCode::BracketRight,
        ';' | ':' => KeyCode::Semicolon,
        '\'' | '"' => KeyCode::Quote,
        ',' | '<' => KeyCode::Comma,
        '.' | '>' => KeyCode::Period,
        '/' | '?' => KeyCode::Slash,
        '\\' | '|' => KeyCode::Backslash,
        '`' | '~' => KeyCode::Backquote,
        ' ' => return (KeyCode::Space, Key::Space),
        '\n' => return (KeyCode::Enter, Key::Enter),
        '\t' => return (KeyCode::Tab, Key::Tab),
        _ => KeyCode::Unidentified(NativeKeyCode::Unidentified),
    };
    (code, Key::Character(ch.to_string().into()))
}

// ============================================================================
// Finding things to aim at
// ============================================================================

/// One interactive UI element as the agent sees it.
struct Element {
    entity: Entity,
    name: String,
    label: String,
    rect: Rect,
    state: UiInteraction,
}

/// Every interactive element currently on screen, top to bottom.
///
/// On screen means laid out, visible, and not entirely clipped away by a scroll
/// view or the window edge. Covered by a popup is still listed: what is on top
/// is picking's question, and the click's reply says who took the press.
fn elements(world: &mut World) -> Vec<Element> {
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .iter(world)
        .next()
        .map(|w| Rect::new(0.0, 0.0, w.physical_width() as f32, w.physical_height() as f32));
    let Some(window) = window else {
        return Vec::new();
    };

    let mut query = world.query::<(
        Entity,
        &UiInteraction,
        &ComputedNode,
        &UiGlobalTransform,
        Option<&InheritedVisibility>,
        Option<&CalculatedClip>,
        Option<&Name>,
    )>();
    let rows: Vec<(Entity, UiInteraction, Rect, String)> = query
        .iter(world)
        .filter(|(.., vis, _, _)| vis.is_none_or(|v| v.get()))
        .filter_map(|(e, state, node, xf, _, clip, name)| {
            let rect = visible_rect(node, xf, clip)?.intersect(window);
            (rect.width() >= 1.0 && rect.height() >= 1.0)
                .then(|| (e, *state, rect, name.map(|n| n.as_str().to_string()).unwrap_or_default()))
        })
        .collect();

    let mut out: Vec<Element> = rows
        .into_iter()
        .map(|(entity, state, rect, name)| Element { entity, label: label_of(world, entity), name, rect, state })
        .collect();
    out.sort_by(|a, b| {
        (a.rect.min.y.round(), a.rect.min.x.round())
            .partial_cmp(&(b.rect.min.y.round(), b.rect.min.x.round()))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

fn visible_rect(node: &ComputedNode, xf: &UiGlobalTransform, clip: Option<&CalculatedClip>) -> Option<Rect> {
    let size = node.size();
    if size.x < 1.0 || size.y < 1.0 {
        return None;
    }
    let mut rect = Rect::from_center_size(xf.translation, size);
    if let Some(clip) = clip {
        if clip.is_fully_clipped() {
            return None;
        }
        for c in clip.rects().unwrap_or_default() {
            let to_world = c.world_to_clip_local.inverse();
            let a = to_world.transform_point2(c.rect.min);
            let b = to_world.transform_point2(c.rect.max);
            rect = rect.intersect(Rect::from_corners(a, b));
        }
    }
    (rect.width() >= 1.0 && rect.height() >= 1.0).then_some(rect)
}

/// What a person would call an element: its text, else its tooltip, else its
/// icon. Text inside a nested interactive element is that element's, not this
/// one's, so the walk stops there; a row would otherwise be named after every
/// button on it.
fn label_of(world: &World, root: Entity) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut icon: Option<String> = None;
    let mut stack = vec![(root, 0u8)];
    while let Some((e, depth)) = stack.pop() {
        if let Some(text) = world.get::<Text>(e) {
            push_words(&mut words, &text.0);
        }
        if let Some(span) = world.get::<TextSpan>(e) {
            push_words(&mut words, &span.0);
        }
        if icon.is_none() && e != root {
            if let Some(name) = world.get::<Name>(e) {
                let n = name.as_str();
                if let Some(rest) = n.strip_prefix("icon:").or_else(|| n.strip_prefix("glyph:")) {
                    icon = Some(rest.to_string());
                }
            }
        }
        if depth >= 10 || words.iter().map(String::len).sum::<usize>() > 90 {
            continue;
        }
        if let Some(children) = world.get::<Children>(e) {
            for child in children.iter().rev() {
                if world.get::<UiInteraction>(child).is_some()
                    || world.get::<InheritedVisibility>(child).is_some_and(|v| !v.get())
                {
                    continue;
                }
                stack.push((child, depth + 1));
            }
        }
    }
    let text = words.join(" ");
    if !text.is_empty() {
        return text;
    }
    if let Some(tip) = world.get::<renzora_ember::widgets::HoverTooltip>(root) {
        return tip.0.clone();
    }
    icon.map(|i| format!("[{i}]")).unwrap_or_default()
}

/// Icon glyphs are private-use characters that mean nothing as text.
fn push_words(words: &mut Vec<String>, text: &str) {
    let clean: String = text.chars().filter(|c| !('\u{E000}'..='\u{F8FF}').contains(c)).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if !clean.is_empty() {
        words.push(clean);
    }
}

fn describe(world: &World, e: Entity) -> String {
    let name = world.get::<Name>(e).map(|n| n.as_str().to_string()).unwrap_or_default();
    let label = label_of(world, e);
    match (name.is_empty(), label.is_empty()) {
        (false, false) => format!("{name} \"{label}\""),
        (false, true) => name,
        (true, false) => format!("\"{label}\""),
        (true, true) => format!("unnamed {}", entity_id(e)),
    }
}

/// Where to aim: `[x, y]` in window pixels, an element id from `ui_find`, or an
/// element's text, tooltip or name. The element's visible centre, plus
/// `offset` pixels when given (to click the left end of a slider, say).
fn resolve_target(world: &mut World, target: Option<&Value>, offset: Option<&Value>) -> Result<(Vec2, String), String> {
    let offset = offset
        .and_then(Value::as_array)
        .and_then(|a| Some(Vec2::new(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)))
        .unwrap_or(Vec2::ZERO);
    match target {
        Some(Value::Array(xy)) => {
            let x = xy.first().and_then(Value::as_f64).ok_or("a point is [x, y]")? as f32;
            let y = xy.get(1).and_then(Value::as_f64).ok_or("a point is [x, y]")? as f32;
            let at = Vec2::new(x, y) + offset;
            Ok((at, format!("the point {}", point(at))))
        }
        Some(Value::String(text)) => {
            let all = elements(world);
            if let Some(bits) = text.trim().parse::<u64>().ok().and_then(Entity::try_from_bits) {
                if let Some(el) = all.iter().find(|el| el.entity == bits) {
                    return Ok((el.rect.center() + offset, describe(world, el.entity)));
                }
                if world.get_entity(bits).is_ok() {
                    return Err(format!("{text} is not an interactive element on screen right now; ui_find lists those"));
                }
            }
            // In tiers, first non-empty wins: what a person reads beats an
            // internal name, and a whole match beats a part. One flat
            // "contains" over both had "Rig" match every `...-trigger`.
            let wanted = text.trim().to_lowercase();
            let label = |el: &&Element| el.label.to_lowercase();
            let name = |el: &&Element| el.name.to_lowercase();
            let tiers: [&dyn Fn(&&Element) -> bool; 4] = [
                &|el| label(el) == wanted,
                &|el| name(el) == wanted,
                &|el| label(el).contains(&wanted),
                &|el| name(el).contains(&wanted),
            ];
            let found: Vec<&Element> =
                tiers.iter().map(|tier| all.iter().filter(|el| tier(el)).collect::<Vec<_>>()).find(|hits| !hits.is_empty()).unwrap_or_default();
            match found.as_slice() {
                [] => Err(format!("nothing on screen is labelled or named \"{text}\"; ui_find lists what is")),
                [one] => Ok((one.rect.center() + offset, describe(world, one.entity))),
                many => {
                    let list: Vec<String> = many.iter().take(10).map(|el| line(el)).collect();
                    Err(format!(
                        "{} elements match \"{text}\"; pass one's id instead:\n{}",
                        many.len(),
                        list.join("\n")
                    ))
                }
            }
        }
        _ => Err("needs a target: [x, y] in screenshot pixels, an element id from ui_find, or an element's text".into()),
    }
}

fn line(el: &Element) -> String {
    let state = match el.state {
        UiInteraction::Pressed => "  [pressed]",
        UiInteraction::Hovered => "  [hovered]",
        UiInteraction::None => "",
    };
    let label = if el.label.is_empty() { String::new() } else { format!("  \"{}\"", el.label) };
    format!(
        "{}  {}{}  at {:.0},{:.0}  {:.0}x{:.0}{}",
        entity_id(el.entity),
        if el.name.is_empty() { "-" } else { &el.name },
        label,
        el.rect.center().x,
        el.rect.center().y,
        el.rect.width(),
        el.rect.height(),
        state
    )
}

/// List what can be clicked.
pub fn ui_find(world: &mut World, args: &Value) -> ToolReply {
    let wanted = args.get("contains").and_then(Value::as_str).map(str::to_lowercase);
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(150) as usize;
    let region = args.get("region").and_then(Value::as_array).and_then(|r| {
        let n = |i: usize| r.get(i).and_then(Value::as_f64).map(|v| v as f32);
        Some(Rect::new(n(0)?, n(1)?, n(0)? + n(2)?, n(1)? + n(3)?))
    });
    let all = elements(world);
    let matching: Vec<&Element> = all
        .iter()
        .filter(|el| {
            wanted.as_ref().is_none_or(|w| el.label.to_lowercase().contains(w) || el.name.to_lowercase().contains(w))
        })
        .filter(|el| region.is_none_or(|r| r.contains(el.rect.center())))
        .collect();
    if matching.is_empty() {
        return ToolReply::Text(format!("no interactive element on screen matches ({} on screen in all)", all.len()));
    }
    let mut out = format!(
        "{} of {} interactive elements on screen. Columns: id, name, \"label\", centre in screenshot pixels, size.\n",
        matching.len().min(limit),
        matching.len()
    );
    for el in matching.iter().take(limit) {
        out.push_str(&line(el));
        out.push('\n');
    }
    if matching.len() > limit {
        out.push_str(&format!("... {} more; narrow with contains or region", matching.len() - limit));
    }
    ToolReply::Text(out)
}

/// Switch workspace by name, or list them.
pub fn set_workspace(world: &mut World, args: &Value) -> ToolReply {
    let Some(mut switch) = world.get_resource_mut::<WorkspaceSwitch>() else {
        return ToolReply::Failed("this editor has no workspace ribbon".into());
    };
    let names = switch.names.clone();
    let active = switch.active.clone().unwrap_or_default();
    let Some(wanted) = args.get("name").and_then(Value::as_str) else {
        return ToolReply::Text(format!("workspaces: {}. Showing: {active}", names.join(", ")));
    };
    let Some(name) = names.iter().find(|n| n.eq_ignore_ascii_case(wanted.trim())) else {
        return ToolReply::Failed(format!("no workspace named {wanted}; there are: {}", names.join(", ")));
    };
    switch.requested = Some(name.clone());
    ToolReply::Text(format!("switching to the {name} workspace (it shows from the next frame)"))
}

// ============================================================================
// The MCP cursor
// ============================================================================

#[derive(Component)]
struct CursorRoot;

#[derive(Component)]
struct CursorRing;

#[derive(Component)]
struct CursorCaption;

fn ink(alpha: f32) -> Color {
    Color::srgba_u8(INK.0, INK.1, INK.2, (alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

#[allow(clippy::type_complexity)]
fn draw_cursor(
    mut commands: Commands,
    player: Res<InputPlayer>,
    windows: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Option<Res<UiScale>>,
    mut roots: Query<(&mut Node, &mut Visibility), With<CursorRoot>>,
    mut rings: Query<(&mut Node, &mut BorderColor, &mut BackgroundColor), (With<CursorRing>, Without<CursorRoot>)>,
    mut captions: Query<&mut Text, With<CursorCaption>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    if roots.is_empty() {
        spawn_cursor(&mut commands);
        return;
    }
    let shown = player.pos.is_some()
        && (!player.jobs.is_empty() || player.last_active.is_some_and(|t| t.elapsed() < LINGER));
    let scale = window.scale_factor() * ui_scale.map(|s| s.0).unwrap_or(1.0);
    for (mut node, mut vis) in &mut roots {
        let want = if shown { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if let Some(pos) = player.pos {
            let (left, top) = (Val::Px(pos.x / scale), Val::Px(pos.y / scale));
            if node.left != left || node.top != top {
                node.left = left;
                node.top = top;
            }
        }
    }
    if !shown {
        return;
    }

    // The ring: a pulse that grows and fades after each press, and a held dot
    // while a button stays down, which is what makes a drag readable.
    let pulse = player.pulse.map(|t| t.elapsed().as_secs_f32() / 0.45).filter(|t| *t < 1.0);
    let (size, border_alpha, fill_alpha) = match (player.pressing, pulse) {
        (true, _) => (16.0, 1.0, 0.45),
        (false, Some(t)) => (10.0 + 34.0 * t, 1.0 - t, 0.0),
        (false, None) => (0.0, 0.0, 0.0),
    };
    for (mut node, mut border, mut fill) in &mut rings {
        let s = Val::Px(size);
        if node.width != s {
            node.width = s;
            node.height = s;
            node.left = Val::Px(-size / 2.0);
            node.top = Val::Px(-size / 2.0);
        }
        *border = BorderColor::all(ink(border_alpha));
        fill.0 = ink(fill_alpha);
    }
    for mut text in &mut captions {
        if text.0 != player.caption {
            text.0 = player.caption.clone();
        }
    }
}

fn spawn_cursor(commands: &mut Commands) {
    let root = commands
        .spawn((
            Name::new("mcp-cursor"),
            CursorRoot,
            HideInHierarchy,
            Node { position_type: PositionType::Absolute, width: Val::Px(0.0), height: Val::Px(0.0), ..default() },
            // Above every popup and menu: the point is to see where it is.
            GlobalZIndex(i32::MAX - 1),
            Visibility::Hidden,
            Pickable::IGNORE,
        ))
        .id();

    let ring = commands
        .spawn((
            CursorRing,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(0.0),
                height: Val::Px(0.0),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BorderColor::all(ink(0.0)),
            BackgroundColor(ink(0.0)),
            Pickable::IGNORE,
        ))
        .id();

    // The arrow, twice: a dark copy a pixel down and right as its shadow, so it
    // reads on light panels and dark ones alike.
    let shadow = renzora_ember::font::glyph(commands, "cursor", (16, 16, 20), 26.0);
    let arrow = renzora_ember::font::glyph(commands, "cursor", INK, 26.0);
    for (glyph, nudge) in [(shadow, 1.5), (arrow, 0.0)] {
        commands.entity(glyph).insert((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(-6.0 + nudge),
                top: Val::Px(-4.0 + nudge),
                ..default()
            },
            Pickable::IGNORE,
        ));
    }

    let caption = commands
        .spawn((
            CursorCaption,
            Text::new(""),
            TextFont { font_size: bevy::text::FontSize::Px(11.0), ..default() },
            TextColor(Color::WHITE),
            Pickable::IGNORE,
        ))
        .id();
    let pill = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                top: Val::Px(20.0),
                padding: UiRect::axes(Val::Px(7.0), Val::Px(3.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(ink(0.92)),
            Pickable::IGNORE,
        ))
        .add_child(caption)
        .id();

    commands.entity(root).add_children(&[ring, shadow, arrow, pill]);
}

// ============================================================================
// Catalogue
// ============================================================================

fn target_arg(description: &str) -> Value {
    json!({
        "description": description,
        "anyOf": [
            { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
            { "type": "string" }
        ]
    })
}

fn offset_arg() -> Value {
    json!({
        "type": "array",
        "items": { "type": "number" },
        "minItems": 2,
        "maxItems": 2,
        "description": "Pixels to add to the target's centre, such as [-40, 0] for the left part of a slider."
    })
}

fn modifiers_arg() -> Value {
    json!({
        "type": "array",
        "items": { "type": "string", "enum": ["ctrl", "shift", "alt", "super"] },
        "description": "Keys held down for the action."
    })
}

/// The input tools' entries for `tools/list`.
pub fn catalogue() -> Value {
    const TARGET: &str = "What to aim at: [x, y] in screenshot pixels, an element id from ui_find, or an element's visible text, tooltip or name.";
    json!([
        {
            "name": "ui_find",
            "description": "The interactive elements on screen right now (buttons, tabs, menu items, fields, sliders, rows), with ids, labels and centres in the same pixels as screenshot. Use it to find what to click.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "contains": { "type": "string", "description": "Only elements whose label or name contains this text, case-insensitively." },
                    "region": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4, "description": "Only elements centred in [x, y, width, height]." },
                    "limit": { "type": "integer", "description": "Most to list (default 150)." }
                }
            }
        },
        {
            "name": "click",
            "description": "Click anything in the editor with a real mouse click: a button, tab, menu item, checkbox, field, a point in the viewport. The orange MCP cursor shows it. The reply names what took the press.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": target_arg(TARGET),
                    "offset": offset_arg(),
                    "button": { "type": "string", "enum": ["left", "right", "middle"], "description": "Default left; right opens context menus." },
                    "count": { "type": "integer", "description": "2 for a double click." },
                    "modifiers": modifiers_arg()
                },
                "required": ["target"]
            }
        },
        {
            "name": "drag",
            "description": "Press, move and release: drag a slider or value, a gizmo handle, a panel tab, an asset onto the viewport, a timeline key, or orbit the camera with the right button.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": target_arg(TARGET),
                    "from_offset": offset_arg(),
                    "to": target_arg("Where to release, in the same forms as from."),
                    "to_offset": offset_arg(),
                    "button": { "type": "string", "enum": ["left", "right", "middle"] },
                    "modifiers": modifiers_arg(),
                    "frames": { "type": "integer", "description": "How many frames the move takes (default by distance)." }
                },
                "required": ["from", "to"]
            }
        },
        {
            "name": "move_mouse",
            "description": "Move the pointer over something without clicking, to show a tooltip or a hover state.",
            "inputSchema": {
                "type": "object",
                "properties": { "target": target_arg(TARGET), "offset": offset_arg() },
                "required": ["target"]
            }
        },
        {
            "name": "scroll",
            "description": "Turn the mouse wheel over something: scroll a list, zoom the viewport or a graph.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": target_arg(TARGET),
                    "offset": offset_arg(),
                    "lines": { "type": "number", "description": "Wheel lines; negative scrolls down or zooms out (default -3)." },
                    "modifiers": modifiers_arg()
                },
                "required": ["target"]
            }
        },
        {
            "name": "press_keys",
            "description": "Press keys or shortcuts, in order: \"ctrl+s\", \"delete\", \"f\", \"escape\", \"ctrl+shift+z\". Goes wherever keyboard focus is, as a real key press does.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keys": {
                        "description": "One chord or a list of them.",
                        "anyOf": [ { "type": "string" }, { "type": "array", "items": { "type": "string" } } ]
                    }
                },
                "required": ["keys"]
            }
        },
        {
            "name": "type_text",
            "description": "Type text into whatever has keyboard focus. Click a field first to focus it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string" },
                    "enter": { "type": "boolean", "description": "Press Enter afterwards." }
                },
                "required": ["text"]
            }
        },
        {
            "name": "set_workspace",
            "description": "Switch the editor to a workspace (Scene, Animation, Materials...) by name. Omit name to list them.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string" } }
            }
        }
    ])
}
