//! The generator panel: service and input on top, options and the Generate
//! button under them, and the list of generations below.
//!
//! Built once when the tab is first shown; everything that changes afterwards
//! is a reactive binding or a `keyed_list`, which is the ember contract.

use bevy::prelude::*;

use renzora_ember::font::{icon_text, ui_font, EmberFonts};
use renzora_ember::interaction::PASS_THROUGH;
use renzora_ember::reactive::tracked::{
    bind_2way, bind_bg, bind_display, bind_text, bind_text_color, bind_with, keyed_list,
};
use renzora_ember::reactive::{KeyedSnapshot, Rx};
use renzora_ember::theme::*;
use renzora_ember::widgets::{
    bind_text_input, button, dropdown, icon_button, segmented, text_input, textarea, toggle_switch,
    WebImages,
};
use renzora_ember::UiInteraction;

use crate::job::{Hub, Mode};
use crate::providers::Provider;
use crate::settings::{grow, Settings};

#[derive(Component)]
pub struct GenerateButton;

#[derive(Component)]
pub struct ChooseImageButton;

/// A job row's cancel button, carrying the job it stops.
#[derive(Component)]
pub struct CancelJob(pub u64);

const THUMB: f32 = 56.0;

fn hub<R: Default>(rx: &Rx, f: impl FnOnce(&Hub) -> R) -> R {
    rx.get_resource::<Hub>().map(f).unwrap_or_default()
}

fn settings<R: Default>(rx: &Rx, f: impl FnOnce(&Settings) -> R) -> R {
    rx.get_resource::<Settings>().map(f).unwrap_or_default()
}

/// Why Generate would refuse right now, or `None` when it would start.
///
/// One function for both the panel's hint line and the press handler, so the
/// button can never say "ready" and then refuse, or the reverse.
pub fn blocker(settings: &Settings, hub: &Hub, has_project: bool) -> Option<String> {
    let provider = settings.provider();
    if !has_project {
        return Some("Open a project first; the model is imported into it.".into());
    }
    if let Some(slot) = provider.key_slot() {
        if settings.key(slot).is_empty() {
            return Some(format!(
                "Add your {} API key in Settings > Plugins > AI Mesh.",
                provider.label()
            ));
        }
    }
    match hub.mode {
        Mode::Text if !provider.takes_text() => Some(format!(
            "{} generates from an image only; switch to Image.",
            provider.label()
        )),
        Mode::Text if hub.prompt.trim().is_empty() => Some("Describe the model to generate.".into()),
        Mode::Image if hub.image_path.trim().is_empty() => {
            Some("Choose a reference image (PNG, JPEG or WebP).".into())
        }
        _ => None,
    }
}

fn blocked(rx: &Rx) -> Option<String> {
    let has_project = rx.get_resource::<renzora::CurrentProject>().is_some();
    match (rx.get_resource::<Settings>(), rx.get_resource::<Hub>()) {
        (Some(s), Some(h)) => blocker(&s, &h, has_project),
        _ => Some(String::new()),
    }
}

pub fn build(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let root = commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(8.0)),
            row_gap: Val::Px(8.0),
            ..default()
        })
        .id();

    let source = source_row(commands, fonts);
    let prompt = prompt_box(commands, fonts);
    let image = image_row(commands, fonts);
    let options = options_row(commands, fonts);
    let action = action_row(commands, fonts);
    let jobs = job_list(commands);
    commands
        .entity(root)
        .add_children(&[source, prompt, image, options, action, jobs]);
    root
}

/// Service dropdown and the Text / Image switch.
fn source_row(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let row = hrow(commands);

    let labels: Vec<&str> = Provider::ALL.iter().map(|p| p.label()).collect();
    let provider = dropdown(commands, fonts, &labels, 0);
    bind_2way(
        commands,
        provider,
        |rx| settings(rx, |s| s.provider().index()),
        |world, index| {
            if let Some(mut s) = world.get_resource_mut::<Settings>() {
                let id = Provider::from_index(*index).id();
                if s.provider != id {
                    s.provider = id.to_string();
                }
            }
        },
    );

    let mode = segmented(commands, &fonts.ui, &["Text", "Image"], 0);
    bind_2way(
        commands,
        mode,
        |rx| hub(rx, |h| h.mode.index()),
        |world, index| {
            if let Some(mut h) = world.get_resource_mut::<Hub>() {
                let mode = Mode::from_index(*index);
                if h.mode != mode {
                    h.mode = mode;
                }
            }
        },
    );

    commands.entity(row).add_children(&[provider, mode]);
    row
}

fn prompt_box(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let input = textarea(
        commands,
        &fonts.ui,
        "Describe the model: shape, material, style. \"A weathered oak barrel with iron bands\"",
        "",
    );
    commands.entity(input).queue(|mut e: EntityWorldMut| {
        if let Some(mut node) = e.get_mut::<Node>() {
            node.width = Val::Percent(100.0);
        }
    });
    bind_text_input(
        commands,
        input,
        |rx| hub(rx, |h| h.prompt.clone()),
        |world, value| {
            if let Some(mut h) = world.get_resource_mut::<Hub>() {
                if h.prompt != value {
                    h.prompt = value;
                }
            }
        },
    );
    bind_display(commands, input, |rx| hub(rx, |h| h.mode == Mode::Text));
    input
}

fn image_row(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let row = hrow(commands);
    let path = text_input(commands, &fonts.ui, "Path to a PNG, JPEG or WebP image", "");
    grow(commands, path);
    bind_text_input(
        commands,
        path,
        |rx| hub(rx, |h| h.image_path.clone()),
        |world, value| {
            if let Some(mut h) = world.get_resource_mut::<Hub>() {
                if h.image_path != value {
                    h.image_path = value;
                }
            }
        },
    );
    let choose = button(commands, &fonts.ui, "Choose...");
    commands.entity(choose).insert(ChooseImageButton);
    commands.entity(row).add_children(&[path, choose]);
    bind_display(commands, row, |rx| hub(rx, |h| h.mode == Mode::Image));
    row
}

/// PBR and low-poly switches, each with its label.
fn options_row(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let row = hrow(commands);
    let pbr = switch(
        commands,
        fonts,
        "PBR materials",
        |s| s.pbr,
        |s, on| s.pbr = on,
    );
    let low = switch(
        commands,
        fonts,
        "Low poly (game-ready)",
        |s| s.low_poly,
        |s, on| s.low_poly = on,
    );
    commands.entity(row).add_children(&[pbr, low]);
    row
}

fn switch(
    commands: &mut Commands,
    fonts: &EmberFonts,
    label: &str,
    get: fn(&Settings) -> bool,
    set: fn(&mut Settings, bool),
) -> Entity {
    let group = hrow(commands);
    let toggle = toggle_switch(commands, get(&Settings::default()));
    bind_2way(
        commands,
        toggle,
        move |rx| settings(rx, get),
        move |world, on| {
            if let Some(mut s) = world.get_resource_mut::<Settings>() {
                if get(&s) != *on {
                    set(&mut s, *on);
                }
            }
        },
    );
    let text = commands
        .spawn((
            Text::new(label),
            ui_font(&fonts.ui, 11.0),
            TextColor(rgb(text_primary())),
        ))
        .id();
    commands.entity(group).add_children(&[toggle, text]);
    group
}

/// Generate, and the line saying why it cannot yet.
fn action_row(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let row = hrow(commands);

    let generate = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(6.0),
                padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(rgb(accent())),
            UiInteraction::default(),
            renzora_ember::cursor_icon::HoverCursor(bevy::window::SystemCursorIcon::Pointer),
            GenerateButton,
            Name::new("ai-mesh-generate"),
        ))
        .id();
    // Dimmed rather than hidden while blocked: the hint beside it says why, and
    // a button that vanishes reads as a panel that failed to load.
    bind_bg(commands, generate, |rx| {
        rgb(if blocked(rx).is_none() { accent() } else { faint_bg() })
    });
    let icon = icon_text(commands, &fonts.phosphor, "magic-wand", on_accent(), 13.0);
    commands.entity(icon).insert(PASS_THROUGH);
    let label = commands
        .spawn((
            Text::new("Generate"),
            ui_font(&fonts.ui, 12.0),
            TextColor(rgb(on_accent())),
            PASS_THROUGH,
        ))
        .id();
    for part in [icon, label] {
        bind_text_color(commands, part, |rx| {
            rgb(if blocked(rx).is_none() { on_accent() } else { text_muted() })
        });
    }
    commands.entity(generate).add_children(&[icon, label]);

    let hint = commands
        .spawn((
            Text::new(""),
            ui_font(&fonts.ui, 10.0),
            TextColor(rgb(text_muted())),
        ))
        .id();
    bind_text(commands, hint, |rx| {
        blocked(rx).unwrap_or_else(|| {
            let running = hub(rx, |h| h.running());
            let provider = settings(rx, |s| s.provider().label().to_string());
            match running {
                0 => format!("Runs on your {provider} account."),
                n => format!("{n} running. Runs on your {provider} account."),
            }
        })
    });

    commands.entity(row).add_children(&[generate, hint]);
    row
}

/// One row per generation, newest first.
fn job_list(commands: &mut Commands) -> Entity {
    let list = commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            ..default()
        })
        .id();
    keyed_list(commands, list, |rx| {
        let ids: Vec<(u64, String, &'static str)> = hub(rx, |h| {
            h.jobs
                .iter()
                .map(|j| (j.id, j.title.clone(), j.provider.label()))
                .collect()
        });
        KeyedSnapshot {
            // Keyed and hashed on the id alone. Progress, stage and thumbnail are
            // the row's own bindings; hashing them here would rebuild the row on
            // every poll and make the bar flicker as it fills.
            items: ids.iter().map(|(id, ..)| (*id, *id)).collect(),
            build: Box::new(move |commands, fonts, i| {
                let (id, title, provider) = &ids[i];
                job_row(commands, fonts, *id, title, provider)
            }),
        }
    });
    list
}

fn job_row(commands: &mut Commands, fonts: &EmberFonts, id: u64, title: &str, provider: &str) -> Entity {
    let card = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(5.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                ..default()
            },
            BackgroundColor(rgb(card_bg())),
            BorderColor::all(rgb(border())),
            Name::new("ai-mesh-job"),
        ))
        .id();

    let thumb = thumbnail(commands, fonts, id);
    let body = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            min_width: Val::Px(0.0),
            row_gap: Val::Px(3.0),
            ..default()
        })
        .id();
    let name = commands
        .spawn((
            Text::new(title.to_string()),
            ui_font(&fonts.ui, 11.0),
            TextColor(rgb(text_primary())),
        ))
        .id();

    let state = commands
        .spawn((
            Text::new(""),
            ui_font(&fonts.mono, 9.0),
            TextColor(rgb(text_muted())),
        ))
        .id();
    let provider = provider.to_string();
    bind_text(commands, state, move |rx| {
        hub(rx, |h| {
            let Some(job) = h.job(id) else { return String::new() };
            let status = match &job.outcome {
                None if job.cancelling() => "cancelling".to_string(),
                None => match job.percent {
                    Some(p) => format!("{} {p}%", job.stage),
                    None => job.stage.clone(),
                },
                Some(_) if job.was_cancelled() => "cancelled".to_string(),
                Some(Ok(_)) if !job.handed_off => "importing".to_string(),
                Some(Ok(path)) => format!("imported: {path}"),
                Some(Err(why)) => why.clone(),
            };
            format!("{provider} \u{b7} {status}")
        })
    });
    bind_text_color(commands, state, move |rx| {
        hub(rx, |h| match h.job(id) {
            // Neutral, not red: a job the user stopped did not fail.
            Some(job) if job.was_cancelled() => rgb(text_muted()),
            Some(job) => match &job.outcome {
                Some(Ok(_)) => rgb(play_green()),
                Some(Err(_)) => rgb(close_red()),
                None => rgb(text_muted()),
            },
            None => rgb(text_muted()),
        })
    });

    let bar = progress_bar(commands, id);
    commands.entity(body).add_children(&[name, state, bar]);

    let cancel = icon_button(commands, fonts, "x");
    commands.entity(cancel).insert(CancelJob(id));
    // Only while there is something to stop, and gone as soon as it is pressed:
    // a second press would do nothing, and a button that stays lit reads as
    // though the first one did not register.
    bind_display(commands, cancel, move |rx| {
        hub(rx, |h| h.job(id).is_some_and(|j| j.running() && !j.cancelling()))
    });

    commands.entity(card).add_children(&[thumb, body, cancel]);
    card
}

/// The provider's preview render once it offers one, over a cube placeholder.
fn thumbnail(commands: &mut Commands, fonts: &EmberFonts, id: u64) -> Entity {
    let frame = commands
        .spawn((
            Node {
                width: Val::Px(THUMB),
                height: Val::Px(THUMB),
                flex_shrink: 0.0,
                position_type: PositionType::Relative,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                overflow: Overflow::clip(),
                border_radius: BorderRadius::all(Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(rgb(hover_bg())),
        ))
        .id();
    let placeholder = icon_text(commands, &fonts.phosphor, "cube", text_muted(), 20.0);
    let image = commands
        .spawn((
            ImageNode::default(),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                display: Display::None,
                ..default()
            },
        ))
        .id();
    // The URL is requested here and the handle read in the same binding:
    // `WebImages::request` is idempotent, and a job's thumbnail arrives exactly
    // once, so this costs one fetch per job rather than a throttle of its own.
    bind_with(
        commands,
        image,
        move |rx| hub(rx, |h| h.job(id).and_then(|j| j.thumbnail.clone())),
        |world, entity, url: &Option<String>| {
            let Some(url) = url else { return };
            if let Some(mut cache) = world.get_resource_mut::<WebImages>() {
                if cache.get(url).is_none() && !cache.failed(url) {
                    cache.request(url);
                }
            }
            world.entity_mut(entity).insert(PendingThumb(url.clone()));
        },
    );
    commands.entity(frame).add_children(&[placeholder, image]);
    frame
}

/// A thumbnail waiting for `WebImages` to finish decoding it. Polled by
/// [`apply_thumbnails`] and removed once the image is shown.
#[derive(Component)]
pub struct PendingThumb(String);

/// Swap a pending thumbnail in once ember's cache has the texture.
///
/// A system rather than a second binding because the binding above runs when
/// the *URL* changes, and the texture lands frames later without the URL
/// changing again.
pub fn apply_thumbnails(
    mut commands: Commands,
    cache: Res<WebImages>,
    mut pending: Query<(Entity, &PendingThumb, &mut ImageNode, &mut Node)>,
) {
    for (entity, thumb, mut image, mut node) in &mut pending {
        if let Some(handle) = cache.get(&thumb.0) {
            image.image = handle;
            node.display = Display::Flex;
            commands.entity(entity).remove::<PendingThumb>();
        } else if cache.failed(&thumb.0) {
            commands.entity(entity).remove::<PendingThumb>();
        }
    }
}

fn progress_bar(commands: &mut Commands, id: u64) -> Entity {
    let track = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(3.0),
                border_radius: BorderRadius::all(Val::Px(2.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(rgb(faint_bg())),
        ))
        .id();
    bind_display(commands, track, move |rx| {
        hub(rx, |h| h.job(id).is_some_and(|j| j.running()))
    });
    let fill = commands
        .spawn((
            Node {
                width: Val::Percent(0.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(rgb(accent())),
        ))
        .id();
    bind_with(
        commands,
        fill,
        move |rx| {
            // A provider with no percentage (fal) shows a third of a bar: enough
            // to say "working" without claiming an amount it does not know.
            hub(rx, |h| h.job(id).map(|j| j.percent.unwrap_or(33)).unwrap_or(0))
        },
        |world, entity, percent: &u8| {
            if let Some(mut node) = world.get_mut::<Node>(entity) {
                node.width = Val::Percent((*percent).max(3) as f32);
            }
        },
    );
    commands.entity(track).add_child(fill);
    track
}

fn hrow(commands: &mut Commands) -> Entity {
    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(8.0),
            row_gap: Val::Px(6.0),
            ..default()
        })
        .id()
}
