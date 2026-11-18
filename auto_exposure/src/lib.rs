//! Renzora Auto Exposure — settings wrapper for Bevy's
//! `bevy_post_process::auto_exposure::AutoExposure`.
//!
//! Bevy's AE is histogram-based with percentile filtering: it builds a
//! 64-bin luminance histogram each frame, ignores the darkest N% and
//! brightest M% of pixels, and animates the camera's exposure so the
//! remaining "metered" pixels average to middle gray. This handles
//! dark/sparse scenes properly (the old log-average implementation
//! would max out exposure when the scene was mostly empty, blowing the
//! frame to white).
//!
//! Because AE *always* targets middle gray, a genuinely dark night scene
//! gets *brightened* (washed out) — correct "eye adaptation", but not what
//! you want for night. The fix is Bevy's exposure-**compensation curve**:
//! it maps metered scene brightness → an exposure offset. We build one that
//! is flat (no change) for bright daytime metering and ramps negative for
//! dark metering, so night stays dark while day is untouched. See
//! [`build_compensation_curve`].
//!
//! This crate just authors user-facing settings on a `WorldEnvironment`
//! source entity and routes them onto every camera via `EffectRouting`.
//! The compute shader, smoothing, percentile filter, and exponential
//! anti-jitter all live in Bevy.
//!
//! Note: Bevy's AE doesn't add itself via `DefaultPlugins` — its
//! `AutoExposurePlugin` is opt-in. We add it from our `build` hook so
//! enabling AutoExposureSettings on any entity Just Works.
//!
//! # Independent of the manual EV100
//!
//! Bevy meters the frame *after* the camera's manual `Exposure` has scaled
//! it, and adds its correction on top. The histogram range and the
//! keep-dark curve are therefore in exposure-scaled units, so the same
//! settings meant different things at different manual EV100s: a scene saved
//! at EV100 9.7 metered 3.8 stops brighter than at 13.5, landed above the
//! keep-dark pivot, got no correction and rendered washed out, while 13.5
//! looked right. The settings are now authored against
//! [`REFERENCE_EV100`], and both the range and the curve are shifted by how
//! far the camera's manual EV100 is from it. That cancels exactly, so with
//! auto exposure on the picture no longer depends on the manual value, which
//! only takes effect when auto exposure is off.

use bevy::camera::Exposure;
use bevy::curve::cubic_splines::LinearSpline;
use bevy::math::vec2;
use bevy::post_process::auto_exposure::{
    AutoExposure, AutoExposureCompensationCurve, AutoExposurePlugin as BevyAePlugin,
};
use bevy::prelude::*;
pub mod inspector;

/// The authored settings live in the **contract crate** now.
///
/// `renzora_level_presets` inserts and queries this component and
/// `renzora_debugger` reads it for the live EV readout — both compiled into the
/// binary, which cannot name a type that lives in a plugin. Re-exported so
/// `renzora_auto_exposure::AutoExposureSettings` still resolves.
pub use renzora::AutoExposureSettings;

/// The manual EV100 the range and keep-dark settings are expressed against:
/// a new camera's default. See the module doc.
pub const REFERENCE_EV100: f32 = 13.5;

/// Cached compensation-curve asset, rebuilt only when the curve-shaping
/// settings change (so we don't churn an asset every frame).
#[derive(Resource, Default)]
struct AeCompensation {
    handle: Handle<AutoExposureCompensationCurve>,
    key: Option<(u32, u32, u32, u32, u32)>,
    /// Stops the camera's manual EV100 sits above [`REFERENCE_EV100`]. The
    /// metered values are this many stops darker than at the reference, so
    /// the curve and the histogram range move down by the same amount.
    offset: f32,
}

/// (Re)build the exposure-compensation curve when its shaping inputs change.
///
/// The curve maps metered scene log-luminance (EV-100, x) → exposure
/// compensation in F-stops (y): flat `0` from `keep_dark_pivot_ev` upward
/// (daytime metering untouched), ramping down to `-strength*(pivot-range_min)`
/// at the dark end so dark/night scenes aren't lifted to middle gray. A larger
/// `keep_dark_strength` darkens night harder; a higher `keep_dark_pivot_ev`
/// pulls more of the dim range down.
fn build_compensation_curve(
    sources: Query<(&AutoExposureSettings, Option<&Exposure>)>,
    scene_cameras: Query<&Exposure, With<renzora::SceneCamera>>,
    mut curves: ResMut<Assets<AutoExposureCompensationCurve>>,
    mut comp: ResMut<AeCompensation>,
) {
    // One AE source drives the look (normally the scene camera); prefer an
    // enabled one, else just the first present.
    let Some((s, own_exposure)) = sources
        .iter()
        .find(|(s, _)| s.enabled)
        .or_else(|| sources.iter().next())
    else {
        return;
    };
    // The manual EV100 the metering happens under: the source's own when it
    // is the camera, else the scene camera's (older scenes keep the settings
    // on the World Environment).
    let manual = own_exposure
        .or_else(|| scene_cameras.iter().next())
        .map_or(REFERENCE_EV100, |e| e.ev100);
    let offset = manual - REFERENCE_EV100;
    let key = (
        s.keep_dark_strength.to_bits(),
        s.keep_dark_pivot_ev.to_bits(),
        s.range_min.to_bits(),
        s.range_max.to_bits(),
        offset.to_bits(),
    );
    if comp.key == Some(key) {
        return;
    }

    // A SINGLE linear segment from `(range_min, comp_lo)` up to `(pivot, 0)`.
    // The compensation curve clamps metered values to its x-range, so this one
    // segment already gives the "flat above the pivot, ramp below" behaviour:
    // metered EV ≥ pivot → 0 (daytime untouched), metered EV ≤ range_min →
    // comp_lo (night kept dark), linear in between. One segment is also what
    // keeps `from_curve` happy — its discontinuity check compares consecutive
    // segment joins with *exact float equality*, which a multi-segment curve
    // (off-round compensation values) trips with `DiscontinuityFound`.
    let lo = s.range_min;
    let top = s.range_max.max(lo + 0.002);
    let pivot = s.keep_dark_pivot_ev.clamp(lo + 0.001, top);
    let strength = s.keep_dark_strength.max(0.0);
    let comp_lo = -strength * (pivot - lo);
    // Built at the reference, then moved along the metering axis only: the
    // compensation for a given scene is the same, it is just read at a
    // different metered value.
    let points = vec![vec2(lo - offset, comp_lo), vec2(pivot - offset, 0.0)];

    match AutoExposureCompensationCurve::from_curve(LinearSpline::new(points)) {
        Ok(curve) => {
            comp.handle = curves.add(curve);
            comp.key = Some(key);
            comp.offset = offset;
        }
        Err(e) => warn!("[auto_exposure] compensation curve build failed: {e:?}"),
    }
}

/// Route AutoExposureSettings from a `WorldEnvironment` source onto
/// every active camera. When disabled, the Bevy `AutoExposure`
/// component is removed and the camera reverts to its static
/// `Exposure.ev100` (typically driven by `TonemappingSettings`).
fn sync_auto_exposure(
    mut commands: Commands,
    sources: Query<(Entity, Ref<AutoExposureSettings>)>,
    routing: Res<renzora::EffectRouting>,
    comp: Res<AeCompensation>,
) {
    let routing_changed = routing.is_changed();
    // Re-apply when the compensation curve was rebuilt too, otherwise a tuning
    // change wouldn't reach the camera until the settings themselves changed.
    let comp_changed = comp.is_changed();
    for (target, source_list) in routing.iter() {
        let mut found = false;
        for &src in source_list {
            if let Ok((_, settings)) = sources.get(src) {
                if !routing_changed && !comp_changed && !settings.is_changed() {
                    found = true;
                    break;
                }
                if settings.enabled {
                    commands.entity(*target).insert(AutoExposure {
                        range: (settings.range_min - comp.offset)..=(settings.range_max - comp.offset),
                        filter: settings.filter_low..=settings.filter_high,
                        speed_brighten: settings.speed_brighten,
                        speed_darken: settings.speed_darken,
                        exponential_transition_distance: settings.exponential_transition_distance,
                        compensation_curve: comp.handle.clone(),
                        ..default()
                    });
                } else {
                    commands.entity(*target).remove::<AutoExposure>();
                }
                found = true;
                break;
            }
        }
        if !found && routing_changed {
            if let Ok(mut ec) = commands.get_entity(*target) {
                ec.remove::<AutoExposure>();
            }
        }
    }
}

/// Route the source camera's manual `Exposure.ev100` onto every viewport
/// target camera. In the editor the entity you select/edit (the scene
/// camera) is the *source*, while the camera that actually renders is a
/// separate viewport *target* — so an edit to `Exposure` on the source
/// has no visible effect unless we copy it across, exactly like bloom and
/// tonemapping are routed. In a shipped game the source *is* the render
/// camera (`src == target`), so we skip — the edit already landed on it,
/// and re-inserting `Exposure` onto itself would spin change-detection
/// forever (same component type in and out, unlike the *Settings wrappers).
fn sync_exposure(
    sources: Query<Ref<Exposure>>,
    routing: Res<renzora::EffectRouting>,
    mut commands: Commands,
) {
    let routing_changed = routing.is_changed();
    for (target, source_list) in routing.iter() {
        for &src in source_list {
            if src == *target {
                break; // source is the render camera itself — nothing to route
            }
            if let Ok(exposure) = sources.get(src) {
                if !routing_changed && !exposure.is_changed() {
                    break;
                }
                commands
                    .entity(*target)
                    .insert(Exposure { ev100: exposure.ev100 });
                break;
            }
        }
    }
}

fn cleanup_auto_exposure(
    mut commands: Commands,
    mut removed: RemovedComponents<AutoExposureSettings>,
    routing: Res<renzora::EffectRouting>,
) {
    if removed.read().next().is_some() {
        for (target, _) in routing.iter() {
            if let Ok(mut ec) = commands.get_entity(*target) {
                ec.remove::<AutoExposure>();
            }
        }
    }
}

/// Mirror the camera's `Exposure.ev100` into `CameraExposureState` so
/// scripting (`camera_ev` Lua/Rhai global) and HUDs can display it.
///
/// Caveat: with Bevy's AE active, the metering result lives in a GPU
/// state buffer, not in `Exposure.ev100` — that component carries the
/// pre-AE baseline (typically what `TonemappingSettings` set). So this
/// reading is the *manual* EV, not the live AE-adjusted value. Fixing
/// that would require a GPU→CPU readback of Bevy's internal state,
/// which is non-trivial; the manual EV is good enough for HUD use.
fn mirror_camera_ev(
    cameras: Query<&Exposure, With<Camera3d>>,
    mut state: ResMut<renzora::core::CameraExposureState>,
) {
    if let Some(exposure) = cameras.iter().next() {
        if (state.ev100 - exposure.ev100).abs() > 1e-4 {
            state.ev100 = exposure.ev100;
        }
    }
}

#[derive(Default)]
pub struct AutoExposurePlugin;

impl Plugin for AutoExposurePlugin {
    fn build(&self, app: &mut App) {
        info!("[runtime] AutoExposurePlugin");
        // Bevy's AutoExposurePlugin is opt-in (not part of DefaultPlugins).
        // Adding it here means `AutoExposure` components are actually
        // honored by the render graph.
        if !app.is_plugin_added::<BevyAePlugin>() {
            app.add_plugins(BevyAePlugin);
        }
        // The compensation-curve asset is normally registered by Bevy's AE
        // plugin; init it defensively (idempotent) so building a curve can't
        // panic on a missing `Assets` resource.
        app.init_asset::<AutoExposureCompensationCurve>();
        app.init_resource::<renzora::core::CameraExposureState>();
        app.init_resource::<AeCompensation>();
        app.register_type::<AutoExposureSettings>();
        // Build the curve before applying AE so a freshly rebuilt curve reaches
        // the camera the same frame.
        app.add_systems(Update, (build_compensation_curve, sync_auto_exposure).chain());
        app.add_systems(Update, (cleanup_auto_exposure, sync_exposure, mirror_camera_ev));

        inspector::register(app);
    }
}

// `Runtime`, explicitly: `plugin!` defaults to `Editor` where `add!` defaulted
// to `Runtime`, so omitting it would stop shipping auto-exposure to games.
renzora::plugin!(AutoExposurePlugin, Runtime);
