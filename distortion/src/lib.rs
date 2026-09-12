//! Animated wave distortion post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `distortion.wgsl`'s `DistortionSettings` must match field for field.
#[post_process(shader = "distortion.wgsl", name = "Distortion", icon = "waves")]
pub struct Distortion {
    #[field(min = 0.0, max = 2.0, speed = 0.01, default = 0.02)]
    pub intensity: f32,
    #[field(min = 0.0, max = 10.0, speed = 0.01, default = 1.0)]
    pub speed: f32,
    #[field(min = 0.1, max = 50.0, speed = 0.1, default = 10.0)]
    pub scale: f32,
    /// Seconds, advanced by [`sync_time`].
    #[field(skip, default = 0.0)]
    pub time: f32,
}

/// Drives the shader's clock.
///
/// Nothing was writing this before, which left the wave frozen mid-cycle: the
/// framework uploads the component's bytes verbatim and interprets no field.
fn sync_time(mut q: Query<&mut Distortion>, time: Res<Time>) {
    for mut s in &mut q {
        s.time += time.delta_secs();
        if s.time > 1024.0 {
            s.time -= 1024.0;
        }
    }
}

#[derive(Default)]
pub struct DistortionPlugin;

impl Plugin for DistortionPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "distortion.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Distortion>::default());
        app.add_systems(Update, sync_time);
        app.register_inspectable::<Distortion>();
    }
}

renzora::plugin!(DistortionPlugin, Runtime);
