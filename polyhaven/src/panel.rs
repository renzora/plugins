//! The browser panel: a toolbar, a live status line, a strip of in-flight
//! downloads, and a thumbnail grid.
//!
//! The content builder runs **once**, when the tab is first activated; every
//! part of it that changes afterwards is a reactive binding or a `keyed_list`.
//! That is the ember contract, and it is what keeps a thousand-entry catalogue
//! from rebuilding its grid on a frame where nothing moved.
//!
//! # Thumbnails come from ember, not from here
//!
//! `WebImages` is ember's async URL → `Handle<Image>` cache, registered
//! unconditionally by `WidgetsPlugin` and pumped every frame by
//! `poll_web_images`. It downloads on a background thread, decodes, and
//! registers an `Image` asset — which is exactly the job a thumbnail grid needs
//! and exactly the job this plugin should not be doing twice. Each tile binds
//! against it and swaps its placeholder for the texture when one arrives, so a
//! thumbnail landing does **not** rebuild the row.

use bevy::prelude::*;

use renzora_ember::font::{icon_text, ui_font, EmberFonts};
use renzora_ember::reactive::tracked::{bind_bg, bind_display, bind_text, bind_text_color, bind_with, keyed_list};
use renzora_ember::reactive::{KeyedSnapshot, Rx};
use renzora_ember::theme::*;
use renzora_ember::widgets::{text_input, WebImages};

use crate::api::{self, Kind};
use crate::hub::{Catalogue, Hub};

/// Tile footprint. Wide enough for a legible 16:10 preview at editor scale, and
/// narrow enough that a docked side panel still fits two across.
const TILE_W: f32 = 132.0;
const THUMB_H: f32 = 84.0;

/// The resolutions offered. Every asset publishes 1k and 2k; 8k is common for
/// textures and HDRIs and rarer for models, where the plan falls back.
const RESOLUTIONS: [&str; 4] = ["1k", "2k", "4k", "8k"];

/// Marks the search field so [`crate::sync_search`] can find the one input whose
/// text drives the filter.
#[derive(Component)]
pub struct SearchBox;

/// A clickable kind tab.
#[derive(Component)]
pub struct KindTab(pub Kind);

/// A clickable resolution chip.
#[derive(Component)]
pub struct ResChip(pub &'static str);

/// Previous / next page. Carries how many pages a press moves by.
#[derive(Component)]
pub struct PageButton(pub i32);

/// A clickable asset tile. Carries the slug so the press handler can find the
/// entry without the tile holding a copy of it.
#[derive(Component)]
pub struct TileAsset(pub String);

/// Read the hub inside a binding, with a default when it is somehow absent.
fn hub<R: Default>(rx: &Rx, f: impl FnOnce(&Hub) -> R) -> R {
    rx.get_resource::<Hub>().map(f).unwrap_or_default()
}

/// Build the panel. Registered as `polyhaven`'s content builder.
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

    let toolbar = toolbar(commands, fonts);
    let status = status_line(commands, fonts);
    let grid = grid(commands, fonts);
    commands.entity(root).add_children(&[toolbar, status, grid]);
    root
}

// ============================================================================
// Toolbar
// ============================================================================

fn toolbar(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let row = commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(6.0),
            row_gap: Val::Px(6.0),
            ..default()
        })
        .id();

    let mut children = Vec::new();
    for kind in Kind::ALL {
        children.push(kind_tab(commands, fonts, kind));
    }

    // Pushes the resolution chips and the search box to the right edge, so the
    // three tabs read as one group rather than as the first three of seven
    // controls.
    children.push(commands.spawn(Node { flex_grow: 1.0, ..default() }).id());

    let res_label = commands
        .spawn((
            Text::new("res"),
            ui_font(&fonts.ui, 10.0),
            TextColor(rgb(text_muted())),
        ))
        .id();
    children.push(res_label);
    for res in RESOLUTIONS {
        children.push(res_chip(commands, fonts, res));
    }

    let search = text_input(commands, &fonts.ui, "Search name, tag or category", "");
    commands.entity(search).insert((
        SearchBox,
        Node {
            width: Val::Px(220.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
    ));
    children.push(search);

    commands.entity(row).add_children(&children);
    row
}

fn kind_tab(commands: &mut Commands, fonts: &EmberFonts, kind: Kind) -> Entity {
    let tab = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(5.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(rgb(faint_bg())),
            Interaction::default(),
            KindTab(kind),
            Name::new("polyhaven-kind"),
        ))
        .id();
    bind_bg(commands, tab, move |rx| {
        let active = hub(rx, |h| h.kind == kind);
        rgb(if active { accent() } else { faint_bg() })
    });

    let icon = icon_text(commands, &fonts.phosphor, kind.icon(), text_primary(), 13.0);
    commands.entity(icon).insert(bevy::ui::FocusPolicy::Pass);
    let label = commands
        .spawn((
            Text::new(kind.label()),
            ui_font(&fonts.ui, 12.0),
            TextColor(rgb(text_primary())),
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();
    // The accent fill is a strong colour; `on_accent` is the palette's answer to
    // what stays readable on top of it, and it is not always `text_primary`.
    for part in [icon, label] {
        bind_text_color(commands, part, move |rx| {
            let active = hub(rx, |h| h.kind == kind);
            rgb(if active { on_accent() } else { text_primary() })
        });
    }
    commands.entity(tab).add_children(&[icon, label]);
    tab
}

fn res_chip(commands: &mut Commands, fonts: &EmberFonts, res: &'static str) -> Entity {
    let chip = commands
        .spawn((
            Node {
                padding: UiRect::axes(Val::Px(7.0), Val::Px(4.0)),
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(rgb(faint_bg())),
            Interaction::default(),
            ResChip(res),
            Name::new("polyhaven-res"),
        ))
        .id();
    bind_bg(commands, chip, move |rx| {
        let active = hub(rx, |h| h.res == res);
        rgb(if active { accent() } else { faint_bg() })
    });

    let label = commands
        .spawn((
            Text::new(res),
            ui_font(&fonts.mono, 10.0),
            TextColor(rgb(text_primary())),
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();
    bind_text_color(commands, label, move |rx| {
        let active = hub(rx, |h| h.res == res);
        rgb(if active { on_accent() } else { text_muted() })
    });
    commands.entity(chip).add_child(label);
    chip
}

// ============================================================================
// Status
// ============================================================================

/// The status line, with the pager pinned to its right end.
///
/// Together rather than in the toolbar, because the two say the same thing: how
/// much of the catalogue you can see, and how to see the rest.
fn status_line(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let row = commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(6.0),
            ..default()
        })
        .id();
    let text = status_text(commands, fonts);
    let spacer = commands.spawn(Node { flex_grow: 1.0, ..default() }).id();
    let prev = page_button(commands, fonts, "caret-left", -1);
    let counter = page_counter(commands, fonts);
    let next = page_button(commands, fonts, "caret-right", 1);
    commands
        .entity(row)
        .add_children(&[text, spacer, prev, counter, next]);
    row
}

/// One line saying what the catalogue is doing and how much of it is showing.
fn status_text(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let text = commands
        .spawn((
            Text::new(""),
            ui_font(&fonts.ui, 11.0),
            TextColor(rgb(text_muted())),
        ))
        .id();
    bind_text(commands, text, |rx| {
        hub(rx, |h| match h.catalogue() {
            Catalogue::Idle => "Opening…".to_string(),
            Catalogue::Loading => format!("Fetching the {} catalogue…", h.kind.label()),
            Catalogue::Failed(why) => format!("Could not load the catalogue: {why}"),
            Catalogue::Ready(all) => {
                let matched = h.visible().len();
                let kind = h.kind.label().to_lowercase();
                if matched == all.len() {
                    format!("{matched} {kind}")
                } else {
                    format!("{matched} of {} {kind}", all.len())
                }
            }
        })
    });
    bind_text_color(commands, text, |rx| {
        hub(rx, |h| match h.catalogue() {
            Catalogue::Failed(_) => rgb(close_red()),
            _ => rgb(text_muted()),
        })
    });
    text
}

/// `2 / 7`. Hidden when there is only one page, so a search that fits on one
/// screen shows no pager at all.
fn page_counter(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let text = commands
        .spawn((
            Text::new(""),
            ui_font(&fonts.mono, 10.0),
            TextColor(rgb(text_muted())),
        ))
        .id();
    bind_text(commands, text, |rx| {
        hub(rx, |h| format!("{} / {}", h.page_index + 1, h.page_count()))
    });
    bind_display(commands, text, |rx| hub(rx, |h| h.page_count() > 1));
    text
}

/// Previous / next. `delta` is the number of pages the press moves by.
///
/// Kept visible but greyed at the ends rather than hidden: a control that
/// disappears when you reach the last page takes the layout with it, and the
/// pair jumping sideways as you page is worse than a dim arrow.
fn page_button(commands: &mut Commands, fonts: &EmberFonts, icon: &str, delta: i32) -> Entity {
    let button = renzora_ember::widgets::icon_button(commands, fonts, icon);
    commands.entity(button).insert(PageButton(delta));
    bind_display(commands, button, |rx| hub(rx, |h| h.page_count() > 1));

    bind_bg(commands, button, move |rx| {
        rgb(if hub(rx, |h| can_turn(h, delta)) {
            faint_bg()
        } else {
            // Flat against the panel: an end-stopped arrow should read as part
            // of the background rather than as something to press.
            panel_bg()
        })
    });
    button
}

/// Is there a page in `delta`'s direction?
fn can_turn(h: &Hub, delta: i32) -> bool {
    if delta < 0 {
        h.page_index > 0
    } else {
        h.page_index + 1 < h.page_count()
    }
}

// ============================================================================
// Grid
// ============================================================================

/// What one tile needs. Built in the snapshot so the build closure owns its data
/// and never reaches back into the World.
///
/// Deliberately holds nothing about the download: progress is read live by the
/// tile's own bindings. If it were a field here, every chunk of every download
/// would change the row's content hash and rebuild the tile — six entities and a
/// fresh `ImageNode` per file, which is how a progress bar ends up making the
/// grid flicker while it fills.
#[derive(Clone)]
struct Row {
    slug: String,
    name: String,
    credit: String,
    thumb: String,
}

fn grid(commands: &mut Commands, _fonts: &EmberFonts) -> Entity {
    let grid = commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::FlexStart,
            column_gap: Val::Px(8.0),
            row_gap: Val::Px(8.0),
            ..default()
        })
        .id();

    keyed_list(commands, grid, |rx| {
        let rows: Vec<Row> = hub(rx, |h| {
            h.page()
                .iter()
                .map(|asset| Row {
                    slug: asset.slug.clone(),
                    name: asset.name.clone(),
                    credit: match (asset.credit(), asset.detail()) {
                        (credit, Some(detail)) if !credit.is_empty() => {
                            format!("{credit} \u{b7} {detail}")
                        }
                        (credit, None) if !credit.is_empty() => credit,
                        (_, Some(detail)) => detail,
                        (_, None) => String::new(),
                    },
                    thumb: api::thumb_url(&asset.slug),
                })
                .collect()
        });
        KeyedSnapshot {
            // Keyed on the slug and hashed on the slug: nothing about a tile
            // changes without the underlying asset changing. The thumbnail and
            // the download state are both handled by the tile's own bindings,
            // and hashing either here would rebuild the whole grid as images
            // trickle in or a progress bar advances.
            items: rows.iter().map(|r| (hash(&r.slug), hash(&r.slug))).collect(),
            build: Box::new(move |commands, fonts, i| tile(commands, fonts, &rows[i])),
        }
    });
    grid
}

fn tile(commands: &mut Commands, fonts: &EmberFonts, row: &Row) -> Entity {
    let card = commands
        .spawn((
            Node {
                width: Val::Px(TILE_W),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(5.0)),
                row_gap: Val::Px(4.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(rgb(card_bg())),
            BorderColor::all(rgb(border())),
            Interaction::default(),
            TileAsset(row.slug.clone()),
            Name::new("polyhaven-tile"),
        ))
        .id();

    let thumb = thumbnail(commands, fonts, row);
    let name = commands
        .spawn((
            Text::new(row.name.clone()),
            ui_font(&fonts.ui, 11.0),
            TextColor(rgb(text_primary())),
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();
    let credit = commands
        .spawn((
            Text::new(if row.credit.is_empty() {
                "Poly Haven \u{b7} CC0".to_string()
            } else {
                row.credit.clone()
            }),
            ui_font(&fonts.ui, 9.0),
            TextColor(rgb(text_muted())),
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();

    let state = state_label(commands, fonts, &row.slug);
    commands.entity(card).add_children(&[thumb, name, credit, state]);
    card
}

/// The one line that says what this asset's download is doing.
///
/// Collapsed when there is no job for the slug, so a tile nobody has clicked is
/// exactly as tall as it was before downloads existed and the grid does not
/// reflow the first time one starts.
fn state_label(commands: &mut Commands, fonts: &EmberFonts, slug: &str) -> Entity {
    let label = commands
        .spawn((
            Text::new(""),
            ui_font(&fonts.mono, 9.0),
            TextColor(rgb(text_muted())),
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();

    let for_display = slug.to_string();
    bind_display(commands, label, move |rx| {
        hub(rx, |h| h.job(&for_display).is_some())
    });

    let for_text = slug.to_string();
    bind_text(commands, label, move |rx| {
        hub(rx, |h| {
            let Some(job) = h.job(&for_text) else {
                return String::new();
            };
            match &job.outcome {
                // `total` is zero until the file list comes back, which is one
                // request on its own: "0/0" would read as stalled.
                None if job.total == 0 => "fetching file list".to_string(),
                None => format!("{}/{} files", job.done, job.total),
                Some(Ok(_)) if job.kind == Kind::Models && !job.handed_off => {
                    "importing".to_string()
                }
                Some(Ok(_)) => "imported".to_string(),
                // The reason, not just "failed" — it is the only place the user
                // ever sees it now that the job strip is gone.
                Some(Err(why)) => why.clone(),
            }
        })
    });

    let for_color = slug.to_string();
    bind_text_color(commands, label, move |rx| {
        hub(rx, |h| match h.job(&for_color).map(|j| &j.outcome) {
            Some(Some(Ok(_))) => rgb(play_green()),
            Some(Some(Err(_))) => rgb(close_red()),
            _ => rgb(text_muted()),
        })
    });
    label
}

/// The preview frame: a muted placeholder icon with the downloaded texture laid
/// over it once ember's cache has one.
///
/// Same shape as ember's own `file_image_tile`, against `WebImages` instead
/// of `FileImages` because the bytes come from a CDN rather than from disk.
fn thumbnail(commands: &mut Commands, fonts: &EmberFonts, row: &Row) -> Entity {
    let frame = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(THUMB_H),
                position_type: PositionType::Relative,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                overflow: Overflow::clip(),
                border_radius: BorderRadius::all(Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(rgb(hover_bg())),
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();

    let placeholder = icon_text(commands, &fonts.phosphor, "image", text_muted(), 22.0);
    commands.entity(placeholder).insert(bevy::ui::FocusPolicy::Pass);
    commands.entity(frame).add_child(placeholder);

    let image = commands
        .spawn((
            ImageNode::default(),
            bevy::ui::FocusPolicy::Pass,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                display: Display::None,
                ..default()
            },
        ))
        .id();
    let url = row.thumb.clone();
    bind_with(
        commands,
        image,
        move |rx| {
            rx.get_resource::<WebImages>()
                .and_then(|cache| cache.get(&url))
        },
        |world, entity, handle: &Option<Handle<Image>>| {
            let Some(handle) = handle else {
                return;
            };
            if let Some(mut node) = world.get_mut::<ImageNode>(entity) {
                if node.image != *handle {
                    node.image = handle.clone();
                }
            }
            if let Some(mut node) = world.get_mut::<Node>(entity) {
                node.display = Display::Flex;
            }
        },
    );
    commands.entity(frame).add_child(image);

    let bar = progress_bar(commands, &row.slug);
    commands.entity(frame).add_child(bar);
    frame
}

/// A thin fill across the bottom of the preview while a download runs.
///
/// Laid over the thumbnail rather than added under the tile: the preview is the
/// part of a tile the eye is already on, and a bar in the card body would change
/// the tile's height the moment a download started and reflow the whole grid.
///
/// Hand-rolled rather than ember's [`progress`](renzora_ember::widgets::progress)
/// because that one takes its value once and hands back only the track. Binding
/// a fill that advances needs the fill entity, which is not something the widget
/// returns.
fn progress_bar(commands: &mut Commands, slug: &str) -> Entity {
    let track = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(0.0),
                height: Val::Px(3.0),
                display: Display::None,
                ..default()
            },
            // Deliberately not a theme colour: this sits on a photograph, and a
            // palette track would vanish against a light one.
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();

    let for_track = slug.to_string();
    bind_display(commands, track, move |rx| {
        // Running only. A finished bar sitting at 100% on the thumbnail says
        // "still working" at a glance; the state label says "imported".
        hub(rx, |h| h.job(&for_track).is_some_and(|job| job.running()))
    });

    let fill = commands
        .spawn((
            Node {
                width: Val::Percent(0.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(rgb(accent())),
            bevy::ui::FocusPolicy::Pass,
        ))
        .id();

    let for_fill = slug.to_string();
    bind_with(
        commands,
        fill,
        move |rx| {
            hub(rx, |h| {
                let Some(job) = h.job(&for_fill) else {
                    return 0.0f32;
                };
                if job.total == 0 {
                    // The file list is still in flight, so there is no
                    // denominator yet. A sliver rather than nothing, so the bar
                    // appears the moment the tile is clicked.
                    return 4.0;
                }
                (job.done as f32 / job.total as f32 * 100.0).clamp(0.0, 100.0)
            })
        },
        |world, entity, percent: &f32| {
            if let Some(mut node) = world.get_mut::<Node>(entity) {
                node.width = Val::Percent(*percent);
            }
        },
    );
    commands.entity(track).add_child(fill);
    track
}

/// Stable 64-bit hash for `keyed_list` keys and content hashes.
fn hash<T: std::hash::Hash>(value: &T) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
