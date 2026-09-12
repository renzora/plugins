//! Heat haze shimmer post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `heat_haze.wgsl`'s `HeatHazeSettings` must match field for field.
#[post_process(shader = "heat_haze.wgsl", name = "Heat Haze", icon = "fire")]
pub struct HeatHaze {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.15)]
    pub intensity: f32,
    #[field(min = 0.1, max = 10.0, speed = 0.1, default = 2.0)]
    pub speed: f32,
    #[field(min = 1.0, max = 100.0, speed = 0.1, default = 20.0)]
    pub scale: f32,
    /// Seconds, advanced by [`sync_time`].
    #[field(skip, default = 0.0)]
    pub time: f32,
}

/// Drives the shader's clock.
///
/// Nothing was writing this before, which left the shimmer frozen: the framework
/// uploads the component's bytes verbatim and interprets no field.
fn sync_time(mut q: Query<&mut HeatHaze>, time: Res<Time>) {
    for mut s in &mut q {
        s.time += time.delta_secs();
        if s.time > 1024.0 {
            s.time -= 1024.0;
        }
    }
}

#[derive(Default)]
pub struct HeatHazePlugin;

impl Plugin for HeatHazePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "heat_haze.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<HeatHaze>::default());
        app.add_systems(Update, sync_time);
        app.register_inspectable::<HeatHaze>();
    }
}

renzora::plugin!(HeatHazePlugin, Runtime);
