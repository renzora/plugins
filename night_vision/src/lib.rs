//! Night vision post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `night_vision.wgsl`'s `NightVisionSettings` must match field for field.
#[post_process(shader = "night_vision.wgsl", name = "Night Vision", icon = "binoculars")]
pub struct NightVision {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 1.0)]
    pub intensity: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.15)]
    pub noise_amount: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.3)]
    pub scanline_amount: f32,
    #[field(min = 1.0, max = 10.0, speed = 0.05, default = 3.0)]
    pub color_amplification: f32,
    /// Seconds, advanced by [`sync_time`].
    #[field(skip, default = 0.0)]
    pub time: f32,
}

/// Drives the shader's clock.
///
/// A post-process effect gets no time for free: the framework uploads the
/// component's bytes verbatim without interpreting a single field. Nothing was
/// writing this before, which left the noise and scanlines frozen into a fixed
/// pattern rather than crawling.
fn sync_time(mut q: Query<&mut NightVision>, time: Res<Time>) {
    for mut s in &mut q {
        // Wrapped rather than assigned from `elapsed_secs`: an f32 second count
        // stops resolving small steps after a few hours of uptime.
        s.time += time.delta_secs();
        if s.time > 1024.0 {
            s.time -= 1024.0;
        }
    }
}

#[derive(Default)]
pub struct NightVisionPlugin;

impl Plugin for NightVisionPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "night_vision.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<NightVision>::default());
        app.add_systems(Update, sync_time);
        app.register_inspectable::<NightVision>();
    }
}

renzora::plugin!(NightVisionPlugin, Runtime);
