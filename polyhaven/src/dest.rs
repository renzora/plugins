//! "Where should this land?" — the destination overlay shown before a download.
//!
//! Built on ember's shared [`folder_picker`], which is the same widget the
//! marketplace's install confirmation and the hierarchy's Create-asset overlay
//! use. Reusing it is not only about consistency: the picker keeps its selection
//! in one [`FolderPick`] resource, so the selected-row highlight is a plain
//! reactive binding rather than click plumbing rewritten per caller, and
//! [`folder_new_button`] means "the folder I want doesn't exist yet" is answered
//! here instead of by cancelling and starting over.
//!
//! # Why a prompt at all
//!
//! The first version wrote every download to a fixed `polyhaven/<kind>/<slug>/`,
//! which is fine right up until a project has its own arrangement — and every
//! project past the first week does. A fixed path also means an asset imported
//! into the wrong place has to be moved by hand afterwards, which breaks every
//! reference the import pipeline just wrote.

use std::path::{Path, PathBuf};

use bevy::prelude::*;

use renzora_ember::font::{ui_font, EmberFonts};
use renzora_ember::theme::*;
use renzora_ember::widgets::{button, folder_new_button, folder_picker, overlay_sized, FolderPick};

use crate::api::Kind;

/// The asset a destination is being chosen for.
///
/// Present exactly while the overlay is up; its absence is what tells the button
/// handler there is nothing to confirm.
#[derive(Resource)]
pub struct PendingPick {
    /// All the confirm handler needs to identify the asset. The display name is
    /// spent building the overlay's header and is not kept: the tile the job
    /// reports back to already has one.
    pub slug: String,
    pub kind: Kind,
    pub res: &'static str,
    pub overlay: Entity,
    /// Where the picker was seeded. Used if `FolderPick` somehow holds nothing —
    /// a stale pick from another overlay is worse than the default we chose.
    pub default_dest: PathBuf,
}

/// The Import button.
#[derive(Component)]
pub struct DestConfirm;

/// The Cancel button, carrying the overlay it dismisses.
#[derive(Component)]
pub struct DestCancel(pub Entity);

/// Where the picker opens unless the user says otherwise.
///
/// The kind's own top-level folder — `models/`, `textures/`, `hdris/` — because
/// that is the engine's convention and where the user is most likely to want it.
///
/// **Only if it already exists.** This creates nothing: a browser that made
/// folders in the project merely by being opened would leave empty directories
/// behind for every tab the user clicked through and never downloaded from. A
/// project without the folder falls back to its root, which always exists and is
/// a row the picker can highlight. The one directory this plugin creates is the
/// asset's own, at the moment there are bytes to write into it.
pub fn default_dest(project_root: &Path, kind: Kind) -> PathBuf {
    let kind_dir = project_root.join(kind.folder());
    if kind_dir.is_dir() {
        kind_dir
    } else {
        project_root.to_path_buf()
    }
}

/// Build and show the overlay for one asset.
pub fn open(
    commands: &mut Commands,
    fonts: &EmberFonts,
    project_root: &Path,
    kind: Kind,
    res: &'static str,
    slug: &str,
    name: &str,
) {
    // Always a folder that exists, so the picker opens with a real row selected
    // rather than highlighting a path that is not in the tree.
    let dest = default_dest(project_root, kind);

    // 480px because the tree is the one thing here that scrolls and has to be
    // given room; everything else is four lines of text.
    let (overlay, content) = overlay_sized(commands, fonts, "Import from Poly Haven", 560.0, 480.0, true);

    let mut kids = Vec::new();
    kids.push(line(
        commands,
        fonts,
        &format!("{name} \u{b7} {} \u{b7} {res}", kind.label().to_lowercase()),
        text_primary(),
        13.0,
    ));
    kids.push(line(commands, fonts, "Import into", text_muted(), 11.0));

    let picker = folder_picker(commands, fonts, project_root, &dest, 1);
    kids.push(picker);

    kids.push(line(
        commands,
        fonts,
        &match kind {
            // Worth stating plainly: the model is not what lands on disk. The
            // glTF is converted and removed, and someone looking for the file
            // they downloaded should know that before they go looking for it.
            Kind::Models => "The glTF and its textures are downloaded, then run through the import \
                             pipeline: you get a .glb with its textures extracted and a .material \
                             per material."
                .to_string(),
            _ => "Files are written into a folder of their own here. Everything on Poly Haven \
                  is CC0, so there is nothing to attribute."
                .to_string(),
        },
        text_muted(),
        10.0,
    ));

    let buttons = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            margin: UiRect::top(Val::Px(8.0)),
            ..default()
        })
        .id();
    // New Folder rides in the button row rather than under the tree — one row of
    // controls, not two. Same placement the marketplace overlay uses.
    let new_folder = folder_new_button(commands, fonts, picker);
    let cancel = button(commands, &fonts.ui, "Cancel");
    commands.entity(cancel).insert(DestCancel(overlay));
    let confirm = button(commands, &fonts.ui, "Import");
    commands
        .entity(confirm)
        .insert((DestConfirm, BackgroundColor(rgb(accent()))));
    commands
        .entity(buttons)
        .add_children(&[new_folder, cancel, confirm]);
    kids.push(buttons);

    let body = commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_grow: 1.0,
            min_height: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            padding: UiRect::all(Val::Px(12.0)),
            ..default()
        })
        .id();
    commands.entity(body).add_children(&kids);
    commands.entity(content).add_child(body);

    commands.insert_resource(PendingPick {
        slug: slug.to_string(),
        kind,
        res,
        overlay,
        default_dest: dest,
    });
}

/// One line of text.
fn line(
    commands: &mut Commands,
    fonts: &EmberFonts,
    text: &str,
    color: (u8, u8, u8),
    size: f32,
) -> Entity {
    commands
        .spawn((
            Text::new(text.to_string()),
            ui_font(&fonts.ui, size),
            TextColor(rgb(color)),
        ))
        .id()
}

/// The folder the overlay settled on: whatever the picker holds, else where it
/// was seeded.
pub fn chosen(pick: &FolderPick, pending: &PendingPick) -> PathBuf {
    pick.path()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| pending.default_dest.clone())
}
