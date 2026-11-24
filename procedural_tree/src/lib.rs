//! Procedural trees — seeded branch and leaf meshes, generated on demand.
//!
//! Insert a [`Tree`] on an entity and the generator builds the branch mesh on
//! that entity plus a child leaf-mesh entity. In the editor, Add Entity →
//! "Procedural Tree".
//!
//! # What changed when this became a native plugin
//!
//! It used to be three crates: `bevy_procedural_tree` (the vendored generator),
//! `renzora_procedural_tree` (the runtime glue) and
//! `renzora_procedural_tree_editor` (the preset and inspector). All three are
//! here now, and the reason is the native-plugin dependency rule: a plugin is
//! compiled by a bare `rustc` against the staged SDK, and the only engine crates
//! it is handed are `bevy`, `renzora` and `renzora::ui`. A workspace crate
//! that depends on Bevy — which the generator does — cannot be reached at all,
//! and cargo is forbidden from resolving one, because a second Bevy compilation
//! would give this plugin different `TypeId`s from the engine and let it read
//! the host's `World` through the wrong layouts.
//!
//! So the generator was **vendored inward** rather than depended on. It sits
//! under [`tree`] as a module tree, still carrying its upstream licences and
//! `README.md` at the plugin root. Its only third-party dependency, `fastrand`,
//! has no Bevy in its graph and is built by cargo into a private rlib.
//!
//! The editor half (the preset and inspector) lives in `src/editor.rs`. It is
//! not behind `#[cfg(feature = "editor")]`, which a native plugin compiled with
//! no cargo features would read as permanently false; `renzora::plugin!`
//! declares the `editor` module itself (the build finds it from
//! `src/editor.rs`), compiling it in the editor and leaving it out of a lean
//! game build instead.

use bevy::prelude::*;

pub mod tree;

use renzora::{HideInHierarchy, WindSway};
use tree::{Leaves, Tree, TreeProceduralGenerationPlugin};

/// Runtime-scope plugin that installs the procedural tree generator.
#[derive(Default)]
pub struct ProceduralTreePlugin;

impl Plugin for ProceduralTreePlugin {
    fn build(&self, app: &mut App) {
        info!("[procedural_tree] native plugin");
        app.add_plugins(TreeProceduralGenerationPlugin);
        app.add_systems(
            Update,
            (tag_generated_leaves, prune_stale_leaves, sway_generated_trees),
        );
    }
}

// ---------------------------------------------------------------------------
// Runtime glue
// ---------------------------------------------------------------------------

/// Tag each tree's generated leaf-mesh child with [`HideInHierarchy`] so it stays
/// out of the outliner and out of scene saves (it's regenerated from the parent
/// `Tree` on load). The branch mesh lives on the parent entity, which stays
/// selectable.
fn tag_generated_leaves(
    mut commands: Commands,
    trees: Query<&Leaves>,
    needs_tag: Query<(), (With<Mesh3d>, Without<HideInHierarchy>)>,
) {
    for leaves in trees.iter() {
        if needs_tag.get(leaves.0).is_ok() {
            commands.entity(leaves.0).insert(HideInHierarchy);
        }
    }
}

/// Give every generated tree its wind response.
///
/// Two different tunings, because they are two different materials on two
/// different meshes: the trunk bends slowly and does not flutter at all, while
/// the leaf canopy is floppier and flutters fully. Sharing one `WindSway`
/// between them would either give the trunk a rustle or take the rustle off the
/// leaves.
///
/// Only ever *inserts*, so a value an author changed in the inspector — or one
/// restored from a scene — is never overwritten on the next frame.
fn sway_generated_trees(
    mut commands: Commands,
    trees: Query<(Entity, &Leaves), With<Tree>>,
    needs_sway: Query<(), Without<WindSway>>,
) {
    for (trunk, leaves) in trees.iter() {
        if needs_sway.get(trunk).is_ok() {
            commands.entity(trunk).insert(WindSway {
                // Wood is stiff, and the trunk mesh's `UV_1` weights already
                // ramp from 0 at the base — this scales what is left.
                response: 0.55,
                flutter: 0.0,
                amplitude: 0.25,
                ..default()
            });
        }
        if needs_sway.get(leaves.0).is_ok() {
            commands.entity(leaves.0).insert(WindSway {
                response: 1.0,
                flutter: 1.0,
                amplitude: 0.4,
                ..default()
            });
        }
    }
}

/// Despawn stray leaf entities with no mesh — the empty husks a pre-tag scene
/// save could leave behind. A freshly generated leaf child always carries a
/// `Mesh3d`, so this only ever removes orphans.
fn prune_stale_leaves(mut commands: Commands, stale: Query<(Entity, &Name), Without<Mesh3d>>) {
    for (entity, name) in stale.iter() {
        if name.as_str() == "ProcGenTreeLeaves" {
            commands.entity(entity).despawn();
        }
    }
}

// `Runtime`, explicitly. `plugin!` defaults to `Editor` where `add!` defaulted
// to `Runtime`, and a tree is scene content a shipped game renders. The editor
// preset and inspector come from `src/editor.rs`, which `plugin!` declares itself.
renzora::plugin!(ProceduralTreePlugin, Runtime);
