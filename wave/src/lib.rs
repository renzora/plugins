//! Sine wave distortion post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `wave.wgsl`'s `WaveSettings` must match field for field.
#[post_process(shader = "wave.wgsl", name = "Wave", icon = "wave-sine")]
pub struct Wave {
    #[field(min = 0.0, max = 0.1, speed = 0.001, default = 0.01)]
    pub amplitude: f32,
    #[field(min = 1.0, max = 50.0, speed = 0.5, default = 10.0)]
    pub frequency: f32,
    #[field(min = 0.1, max = 10.0, speed = 0.1, default = 2.0)]
    pub speed: f32,
    /// Seconds, advanced by [`sync_time`].
    #[field(skip, default = 0.0)]
    pub time: f32,
}

/// Drives the shader's clock.
///
/// Nothing was writing this before, which left the wave standing still: the
/// framework uploads the component's bytes verbatim and interprets no field.
fn sync_time(mut q: Query<&mut Wave>, time: Res<Time>) {
    for mut s in &mut q {
        s.time += time.delta_secs();
        if s.time > 1024.0 {
            s.time -= 1024.0;
        }
    }
}

#[derive(Default)]
pub struct WavePlugin;

impl Plugin for WavePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "wave.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Wave>::default());
        app.add_systems(Update, sync_time);
        app.register_inspectable::<Wave>();
    }
}

renzora::plugin!(WavePlugin, Runtime);
