//! Vignette — a thin wrapper around **Bevy 0.19's built-in `Vignette`**
//! (`bevy::post_process`), shipped as a native plugin.
//!
//! Follows the same settings→sync→camera pattern as the other bevy-built-in
//! wrappers: `VignetteSettings` is authored on a `WorldEnvironment`-style entity
//! and routed onto cameras by `sync_vignette`, which toggles bevy's `Vignette`
//! component. The bevy post-process effect stack renders it.
//!
//! ## Where the inspector lives
//!
//! In-workspace this crate gated `inspector_entry()` behind a cargo `editor`
//! feature. A native plugin has no features to gate on: `rustc` compiles it
//! directly against the SDK with none set, so `#[cfg(feature = "editor")]` would
//! be false and the Vignette section would silently vanish from the inspector.
//! The entry lives in `src/editor.rs` instead: `renzora::plugin!` declares the
//! `editor` module itself (the build finds it from `src/editor.rs`), compiling
//! it in the editor and leaving it out of a lean game build.
//!
//! Declared `Runtime` so it loads in the editor viewport *and* a copy-based
//! export. `plugin!` defaults to `Editor`, which would have shipped no vignette
//! at all.

use bevy::post_process::effect_stack::Vignette;
use bevy::prelude::*;

use renzora::serde::{Deserialize, Serialize};

/// Authored vignette settings, routed to cameras as a bevy `Vignette`.
///
/// Note `#[serde(crate = "renzora::serde")]`. A plugin must derive against the
/// **engine's** serde, not one cargo resolved for it — `Vec3` implements the
/// engine's `Serialize`, so a private copy fails to compile. See
/// [`renzora::serde`] for the whole story; the short version is that a plugin
/// needs the engine's serde the same way it needs the engine's `Transform`.
#[derive(Component, Clone, Debug, Reflect, Serialize, Deserialize)]
#[serde(crate = "renzora::serde")]
#[reflect(Component, Serialize, Deserialize)]
pub struct VignetteSettings {
    pub enabled: bool,
    /// Strength of the darkening at the edges.
    pub intensity: f32,
    /// Radius (0..1) at which the vignette starts.
    pub radius: f32,
    /// Falloff softness from the radius to the corners.
    pub smoothness: f32,
    /// 0 = elliptical (follows aspect), 1 = circular.
    pub roundness: f32,
    /// Vignette tint (linear RGB; black is the classic look).
    pub color: Vec3,
    /// Compensates the darkening in the very corners.
    pub edge_compensation: f32,
}

impl Default for VignetteSettings {
    // Mirrors `bevy::post_process::Vignette::default()`.
    fn default() -> Self {
        Self {
            enabled: true,
            intensity: 1.0,
            radius: 0.75,
            smoothness: 5.0,
            roundness: 1.0,
            color: Vec3::ZERO,
            edge_compensation: 1.0,
        }
    }
}

fn vignette_from(s: &VignetteSettings) -> Vignette {
    Vignette {
        intensity: s.intensity,
        radius: s.radius,
        smoothness: s.smoothness,
        roundness: s.roundness,
        color: Color::srgb(s.color.x, s.color.y, s.color.z),
        edge_compensation: s.edge_compensation,
        center: Vec2::new(0.5, 0.5),
    }
}

fn sync_vignette(
    mut commands: Commands,
    sources: Query<(Entity, Ref<VignetteSettings>)>,
    routing: Res<renzora::EffectRouting>,
) {
    let routing_changed = routing.is_changed();
    for (target, source_list) in routing.iter() {
        let mut found = false;
        for &src in source_list {
            if let Ok((_, settings)) = sources.get(src) {
                if !routing_changed && !settings.is_changed() {
                    found = true;
                    break;
                }
                if settings.enabled {
                    commands.entity(*target).insert(vignette_from(&settings));
                } else {
                    commands.entity(*target).remove::<Vignette>();
                }
                found = true;
                break;
            }
        }
        if !found && routing_changed {
            if let Ok(mut ec) = commands.get_entity(*target) {
                ec.remove::<Vignette>();
            }
        }
    }
}

fn cleanup_vignette(
    mut commands: Commands,
    mut removed: RemovedComponents<VignetteSettings>,
    routing: Res<renzora::EffectRouting>,
) {
    if removed.read().next().is_some() {
        for (target, _) in routing.iter() {
            if let Ok(mut ec) = commands.get_entity(*target) {
                ec.remove::<Vignette>();
            }
        }
    }
}

#[derive(Default)]
pub struct VignettePlugin;

impl Plugin for VignettePlugin {
    fn build(&self, app: &mut App) {
        info!("[vignette] native plugin (bevy built-in)");
        app.register_type::<VignetteSettings>();
        app.add_systems(Update, (sync_vignette, cleanup_vignette));
    }
}

renzora::plugin!(VignettePlugin, Runtime);
