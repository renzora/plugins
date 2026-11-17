//! Render Toggles probe: debug switches that turn off expensive,
//! resolution-bound render work so its cost can be isolated without a rebuild.
//!
//! Every switch defaults to on (normal rendering). Turning one off forces that
//! work off every frame; turning it back on restores what was there. These are
//! diagnostic switches, not settings: nothing here is written to disk.
//!
//! Why this exists: on an empty scene the editor still spent about 30 ms of
//! GPU a frame for about 15 draw calls, because the cost is fullscreen passes
//! on the active camera (atmosphere, auto-exposure), not geometry. These let
//! you bisect which pass is responsible on a given machine.
//!
//! Two kinds of switch live here. Preview cameras and auto-exposure are this
//! plugin's own [`RenderToggles`], enforced by the systems [`register`]
//! installs. Shadows, meshes, textures and lighting write the engine's
//! `ViewportSettings.render_toggles`, which already has a single authority in
//! `renzora_viewport`, so keeping a copy here would only fight it.
//!
//! Prepasses are intentionally not switchable: their attachment layout is
//! fixed at camera spawn (re-adding trips a wgpu validation error that degrades
//! the pipeline until restart), and the depth prepass is a win in real scenes.

use bevy::prelude::*;

use renzora::core::viewport_types::{self, ViewportSettings};
use renzora::lang::t;
use renzora::probes::{ProbeDef, ProbeItem, ProbeLevel, ProbeReport};
use renzora::{AutoExposureSettings, IsolatedCamera, SplashState};

/// The plugin-owned switches. `true` is normal rendering; the `ae_saved` list
/// holds each auto-exposure source's own `enabled` so switching back is fully
/// reversible.
#[derive(Resource)]
pub struct RenderToggles {
    /// Allow offscreen preview cameras (Studio, Material, Shader, Particle,
    /// thumbnails, the UI render camera: anything `IsolatedCamera`) to render.
    pub preview_cameras: bool,
    /// Auto-exposure post-process (the per-frame histogram pass).
    pub auto_exposure: bool,
    ae_saved: Vec<(Entity, bool)>,
}

impl Default for RenderToggles {
    fn default() -> Self {
        Self { preview_cameras: true, auto_exposure: true, ae_saved: Vec::new() }
    }
}

/// Install the resource and the enforcement systems.
///
/// Not gated on the probe being watched, deliberately: these systems enforce
/// the switches on the live cameras, and the probe is only the control that
/// sets them. Gating them would mean closing the companion silently undid what
/// you had just turned off, because the editor's own Update-stage camera
/// activation and effect routing would re-apply unopposed. PostUpdate so they
/// win over those Update systems and run before render extraction.
pub fn register(app: &mut App) {
    app.init_resource::<RenderToggles>();
    app.add_systems(
        PostUpdate,
        (enforce_preview_cameras, enforce_auto_exposure).run_if(in_state(SplashState::Editor)),
    );
}

pub fn def() -> ProbeDef {
    ProbeDef {
        id: "render_toggles",
        title: "Render Toggles",
        icon: "sliders",
        category: "Rendering",
        build,
        act: Some(act),
    }
}

// Stable control ids. The companion sends these back to `act`.
const PREVIEW_CAMERAS: &str = "preview_cameras";
const AUTO_EXPOSURE: &str = "auto_exposure";
const SHADOWS: &str = "shadows";
const MESHES: &str = "meshes";
const TEXTURES: &str = "textures";
const LIGHTING: &str = "lighting";

fn build(world: &mut World) -> ProbeReport {
    let mut report = ProbeReport::default();
    report.section(t("render.toggles_title"));
    report.item(ProbeItem::note(t("render.toggles_hint"), ProbeLevel::Muted));

    // Both read as on when their resource is missing, as the panel did: off
    // must only ever mean somebody switched it off.
    let (preview, ae) = world
        .get_resource::<RenderToggles>()
        .map(|t| (t.preview_cameras, t.auto_exposure))
        .unwrap_or((true, true));
    let vp = world
        .get_resource::<ViewportSettings>()
        .map(|s| {
            let r = &s.render_toggles;
            (r.shadows, r.mesh, r.textures, r.lighting)
        })
        .unwrap_or((true, true, true, true));

    report
        .item(ProbeItem::toggle(PREVIEW_CAMERAS, t("render.toggle_preview_cameras"), preview))
        .item(ProbeItem::toggle(AUTO_EXPOSURE, t("render.toggle_auto_exposure"), ae))
        .item(ProbeItem::toggle(SHADOWS, t("render.toggle_shadows"), vp.0))
        // Meshes off discards every mesh pixel: the cleanest safe way to blank
        // the scene geometry.
        .item(ProbeItem::toggle(MESHES, t("render.toggle_meshes"), vp.1))
        .item(ProbeItem::toggle(TEXTURES, t("render.textures"), vp.2))
        .item(ProbeItem::toggle(LIGHTING, t("render.toggle_lighting"), vp.3));

    if world.get_resource::<ViewportSettings>().is_none() {
        report.item(ProbeItem::note(
            "ViewportSettings is not present: Shadows, Meshes, Textures and Lighting do nothing in this process.",
            ProbeLevel::Warn,
        ));
    }
    report
}

/// Flip a switch, writing the same resource field the panel's checkbox wrote.
/// A button press (no value) is ignored: every control here is a toggle.
fn act(world: &mut World, id: &str, value: Option<bool>) {
    let Some(v) = value else {
        return;
    };
    match id {
        PREVIEW_CAMERAS => set(world, |t| t.preview_cameras = v),
        AUTO_EXPOSURE => set(world, |t| t.auto_exposure = v),
        SHADOWS => vp_set(world, |t| t.shadows = v),
        MESHES => vp_set(world, |t| t.mesh = v),
        TEXTURES => vp_set(world, |t| t.textures = v),
        LIGHTING => vp_set(world, |t| t.lighting = v),
        _ => {}
    }
}

fn set(world: &mut World, f: impl FnOnce(&mut RenderToggles)) {
    if let Some(mut t) = world.get_resource_mut::<RenderToggles>() {
        f(&mut t);
    }
}

/// Mutate a `ViewportSettings.render_toggles` flag; the engine applies it.
fn vp_set(world: &mut World, f: impl FnOnce(&mut viewport_types::RenderToggles)) {
    if let Some(mut s) = world.get_resource_mut::<ViewportSettings>() {
        f(&mut s.render_toggles);
    }
}

/// While off, force every offscreen preview camera inactive. While on, do
/// nothing: the owning editor systems re-activate them as panels mount.
fn enforce_preview_cameras(
    toggles: Res<RenderToggles>,
    mut cams: Query<&mut Camera, With<IsolatedCamera>>,
) {
    if toggles.preview_cameras {
        return;
    }
    for mut cam in &mut cams {
        if cam.is_active {
            cam.is_active = false;
        }
    }
}

/// Disable the auto-exposure sources while off, restore them on the way back.
fn enforce_auto_exposure(
    mut toggles: ResMut<RenderToggles>,
    mut last: Local<Option<bool>>,
    mut ae: Query<(Entity, &mut AutoExposureSettings)>,
) {
    let now = toggles.auto_exposure;
    let transitioned = *last != Some(now);

    if !now {
        if transitioned {
            toggles.ae_saved.clear();
            for (e, s) in &ae {
                toggles.ae_saved.push((e, s.enabled));
            }
        }
        for (_, mut s) in &mut ae {
            if s.enabled {
                s.enabled = false;
            }
        }
    } else if transitioned {
        for (e, en) in std::mem::take(&mut toggles.ae_saved) {
            if let Ok((_, mut s)) = ae.get_mut(e) {
                s.enabled = en;
            }
        }
    }

    *last = Some(now);
}
