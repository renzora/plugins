//! Poly Haven inside the editor: browse the CC0 library and import an asset
//! into the open project without leaving the dock.
//!
//! One panel (`polyhaven`, under **Assets** in the Add-Panel picker) listing the
//! three catalogues — HDRIs, textures, models — as a searchable thumbnail grid.
//! Clicking a tile downloads that asset at the chosen resolution into
//! `assets/polyhaven/<kind>/<slug>/`, where the asset browser and the glTF
//! importer pick it up like anything else on disk.
//!
//! # Why this is a native plugin and not a C-ABI one
//!
//! Two reasons, and the first is decisive. The C-ABI HTTP domain delivers a
//! response as a `String` — the host runs `response.text()` and the bytes reach
//! the plugin through `String::from_utf8_lossy` — so a `.hdr`, a `.jpg` or a
//! `.bin` arrives with every invalid sequence replaced by U+FFFD. Every file
//! this plugin exists to fetch is binary, so that boundary cannot carry them at
//! all. `renzora::net::Request` hands back `Vec<u8>`, and a native plugin links
//! the contract crate that defines it.
//!
//! The second is that an asset browser is editor furniture. It never ships
//! inside a game, so the thing a C-ABI plugin buys — running in an export, on
//! wasm, on a console — is not something this would ever use.
//!
//! # What it deliberately does not do
//!
//! No conversion, no material authoring, no scene insertion. The files land in
//! the project and the engine's existing importers own everything after that.
//! A browser that also built `StandardMaterial`s would be making decisions that
//! belong to the material editor, and would go stale the moment it changed.

mod api;
mod dest;
mod hub;
mod panel;

use std::collections::HashSet;

use bevy::prelude::*;

use renzora::RenzoraShellExt;
use renzora_ember::font::EmberFonts;
use renzora_ember::panel::RegisterPanelContent;
use renzora_ember::widgets::{EmberTextInput, FolderPick, WebImages};

use api::Kind;
use hub::{Catalogue, Hub, Job};
use panel::{KindTab, PageButton, ResChip, SearchBox, TileAsset};

/// Thumbnail fetches started but not yet resolved, capped so opening a
/// thousand-entry catalogue does not start a thousand downloads.
///
/// `WebImages::request` spawns a thread per URL, so the ceiling is not a
/// politeness setting — an unthrottled first frame would put one thread per
/// visible asset on the CDN at once. The count is recomputed from the cache each
/// frame rather than tracked incrementally: a fetch that fails is remembered by
/// `WebImages::failed`, so a lost response cannot leak a slot forever.
#[derive(Resource, Default)]
struct Thumbs {
    requested: HashSet<String>,
}

/// How many thumbnail downloads may be outstanding at once.
const MAX_THUMBS_IN_FLIGHT: usize = 12;

pub struct PolyHavenPlugin;

impl Plugin for PolyHavenPlugin {
    fn build(&self, app: &mut App) {
        info!("[editor] PolyHavenPlugin");
        app.init_resource::<Hub>();
        app.init_resource::<Thumbs>();
        // `renzora_import_ui` also inits this, and `init_resource` is
        // insert-if-absent, so both are correct and neither can clobber the
        // other's queue. Doing it here too removes an ordering assumption: a
        // native plugin loads AFTER every in-workspace plugin has built today,
        // but a `ResMut` of a resource that merely happens to exist by then is
        // a panic waiting for that order to change.
        app.init_resource::<renzora::core::ImportInPlaceQueue>();

        // The Add-Panel picker entry. Without this the panel exists but there is
        // no way to open it.
        app.register_shell_panel("polyhaven", "Poly Haven", "cloud-arrow-down", "Assets");

        app.register_panel_content("polyhaven", true, panel::build)
            // Everything here is panel-active gated by `PanelScope::systems`,
            // which is right for all of it: with the tab hidden there is nothing
            // to search, no tile to click and no thumbnail on screen to fetch.
            .systems(
                Update,
                (
                    kick_catalogue,
                    sync_search,
                    request_thumbs,
                    on_kind_tab,
                    on_res_chip,
                    on_page_button,
                    on_tile_pressed,
                ),
            )
            // The exceptions, and the case `always` exists for. A download in
            // flight has to keep landing while the user works in another tab —
            // gating that would freeze the progress chips and leave the files
            // half-written until they came back. And a modal whose buttons stop
            // being polled is a modal with no way out.
            .always(Update, (drain_inbox, on_dest_buttons));
    }
}

/// Fetch the visible tab's catalogue the first time it is looked at.
///
/// Idle → Loading happens here rather than in the worker so a slow request
/// cannot be started twice: the state moves before the thread does.
fn kick_catalogue(mut hub: ResMut<Hub>) {
    let kind = hub.kind;
    if !matches!(hub.catalogues[kind.index()], Catalogue::Idle) {
        return;
    }
    hub.catalogues[kind.index()] = Catalogue::Loading;
    let inbox = hub.inbox.clone();
    hub::spawn_catalogue_fetch(kind, inbox);
}

/// Copy the search box's text into the hub as lowercased terms.
///
/// Split into terms here rather than in the filter: the filter runs once per
/// entry per snapshot, and re-splitting the same string a thousand times would
/// be the most expensive thing in the panel.
fn sync_search(mut hub: ResMut<Hub>, boxes: Query<&EmberTextInput, With<SearchBox>>) {
    let Ok(input) = boxes.single() else {
        return;
    };
    let terms: Vec<String> = input
        .value
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .collect();
    // `ResMut` marks the resource changed on any deref, and every binding in the
    // panel is gated on that — so writing an unchanged value every frame would
    // re-run every snapshot in the grid forever.
    if hub.terms != terms {
        hub.terms = terms;
        // A new filter is a new list. Page 3 of the old results is not a
        // meaningful place to be in the new ones.
        hub.reset_page();
    }
}

/// Top the thumbnail queue up to [`MAX_THUMBS_IN_FLIGHT`], nearest the top of
/// the grid first.
fn request_thumbs(hub: Res<Hub>, mut thumbs: ResMut<Thumbs>, mut cache: ResMut<WebImages>) {
    let mut in_flight = thumbs
        .requested
        .iter()
        .filter(|url| cache.get(url).is_none() && !cache.failed(url))
        .count();
    if in_flight >= MAX_THUMBS_IN_FLIGHT {
        return;
    }

    let wanted: Vec<String> = hub
        .page()
        .iter()
        .map(|asset| api::thumb_url(&asset.slug))
        .collect();
    for url in wanted {
        if in_flight >= MAX_THUMBS_IN_FLIGHT {
            break;
        }
        if thumbs.requested.contains(&url) {
            continue;
        }
        cache.request(&url);
        thumbs.requested.insert(url);
        in_flight += 1;
    }
}

/// Switch tabs.
fn on_kind_tab(mut hub: ResMut<Hub>, tabs: Query<(&Interaction, &KindTab), Changed<Interaction>>) {
    for (interaction, tab) in &tabs {
        if *interaction == Interaction::Pressed && hub.kind != tab.0 {
            hub.kind = tab.0;
            // The new tab has its own list and its own length. Staying on page 5
            // of the one you left is how a tab looks like it loaded nothing.
            hub.reset_page();
        }
    }
}

/// Page the grid backwards or forwards.
fn on_page_button(
    mut hub: ResMut<Hub>,
    buttons: Query<(&Interaction, &PageButton), Changed<Interaction>>,
) {
    for (interaction, button) in &buttons {
        if *interaction == Interaction::Pressed {
            // `turn_page` clamps and writes only on a real change, so a press at
            // either end costs nothing and does not mark the hub dirty.
            hub.turn_page(button.0);
        }
    }
}

/// Change the download resolution.
fn on_res_chip(mut hub: ResMut<Hub>, chips: Query<(&Interaction, &ResChip), Changed<Interaction>>) {
    for (interaction, chip) in &chips {
        if *interaction == Interaction::Pressed && hub.res != chip.0 {
            hub.res = chip.0;
        }
    }
}

/// Ask where the clicked tile should land.
///
/// The press opens the destination overlay rather than starting a download: a
/// fixed path is fine until a project has its own arrangement, and moving an
/// asset afterwards breaks every reference the import pipeline wrote for it.
fn on_tile_pressed(
    mut commands: Commands,
    hub: Res<Hub>,
    fonts: Option<Res<EmberFonts>>,
    tiles: Query<(&Interaction, &TileAsset), Changed<Interaction>>,
    project: Option<Res<renzora::CurrentProject>>,
    pending: Option<Res<dest::PendingPick>>,
) {
    // One overlay at a time — it is modal, and `FolderPick` is a single shared
    // resource, so a second would silently take over the first one's selection.
    if pending.is_some() {
        return;
    }
    let Some(fonts) = fonts else {
        return;
    };

    for (interaction, tile) in &tiles {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let slug = tile.0.clone();

        // Already fetched or fetching. Re-downloading on a second click would
        // overwrite files the project may already reference, for no gain.
        if hub.job(&slug).is_some() {
            continue;
        }

        let Some(project) = project.as_deref() else {
            warn!("[polyhaven] no project is open, so there is nowhere to import to");
            continue;
        };

        let Catalogue::Ready(assets) = hub.catalogue() else {
            continue;
        };
        let Some(asset) = assets.iter().find(|a| a.slug == slug) else {
            continue;
        };

        dest::open(
            &mut commands,
            &fonts,
            &project.path,
            hub.kind,
            hub.res,
            &asset.slug,
            &asset.name,
        );
        // One press, one overlay: the query can yield several tiles in a frame
        // if the pointer crossed them, and every one after this would see no
        // `PendingPick` yet because the insert is a queued command.
        break;
    }
}

/// Confirm or cancel the destination overlay.
///
/// Kept out of the panel-active gate. The overlay is modal, so the dock cannot
/// be reached while it is up — but a gate that stopped these handlers would
/// leave a modal on screen whose buttons do nothing, and there would be no way
/// back. That is the failure worth spending an ungated query on.
fn on_dest_buttons(
    mut commands: Commands,
    mut hub: ResMut<Hub>,
    confirm: Query<&Interaction, (With<dest::DestConfirm>, Changed<Interaction>)>,
    cancel: Query<(&Interaction, &dest::DestCancel), Changed<Interaction>>,
    pending: Option<Res<dest::PendingPick>>,
    pick: Res<FolderPick>,
    project: Option<Res<renzora::CurrentProject>>,
) {
    for (interaction, button) in &cancel {
        if *interaction == Interaction::Pressed {
            commands.entity(button.0).despawn();
            commands.remove_resource::<dest::PendingPick>();
            return;
        }
    }

    if !confirm.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    let Some(pending) = pending else {
        return;
    };

    // Each asset takes its own folder under whatever was picked: a model brings
    // a `.bin` and a `textures/` beside it, and a texture brings five maps, so
    // dropping them straight into a shared folder would interleave assets that
    // have to be moved together.
    let folder = dest::chosen(&pick, &pending).join(&pending.slug);
    let label = project
        .as_deref()
        .and_then(|p| p.make_relative(&folder))
        .unwrap_or_else(|| folder.display().to_string());

    hub.jobs.insert(
        0,
        Job {
            slug: pending.slug.clone(),
            kind: pending.kind,
            folder: folder.clone(),
            done: 0,
            total: 0,
            outcome: None,
            handed_off: false,
        },
    );
    hub::spawn_download(
        pending.kind,
        pending.res,
        pending.slug.clone(),
        folder,
        label,
        hub.inbox.clone(),
    );

    commands.entity(pending.overlay).despawn();
    commands.remove_resource::<dest::PendingPick>();
}

/// Fold whatever the worker threads finished into the hub, and hand finished
/// model downloads to the import pipeline.
fn drain_inbox(mut hub: ResMut<Hub>, mut imports: ResMut<renzora::core::ImportInPlaceQueue>) {
    let messages = hub.inbox.drain();
    if messages.is_empty() {
        // Same reason `sync_search` compares first: an unconditional `ResMut`
        // deref would mark the hub changed on every frame of an idle editor and
        // defeat every dependency gate in the panel.
        return;
    }
    for message in messages {
        hub.apply(message);
    }

    // A downloaded glTF is not an imported model: without this it loads
    // untextured, with no extracted `textures/` and no `.material` per material.
    // The pipeline lives in `renzora_import`, which a native plugin cannot link,
    // so the request crosses as a path through the contract crate.
    //
    // Models only. A texture or an HDRI is an image the engine already loads
    // from disk, and running one through a model importer would find nothing to
    // convert.
    for job in hub.jobs.iter_mut() {
        if job.handed_off || job.kind != Kind::Models {
            continue;
        }
        if matches!(job.outcome, Some(Ok(_))) {
            imports.0.push(job.folder.clone());
            job.handed_off = true;
        }
    }
}

// `plugin!` rather than `add!`: this is a native plugin, compiled against the
// staged SDK and loaded from `plugins/` at startup, not an rlib the build
// generator links into the editor binary. `Editor` is the default and is what
// this wants — a shipped game has no dock to put a browser in, and no project
// directory to import into.
renzora::plugin!(PolyHavenPlugin);
