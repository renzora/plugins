//! Digital glitch post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `glitch.wgsl`'s `GlitchSettings` must match field for field.
#[post_process(shader = "glitch.wgsl", name = "Glitch", icon = "wave-square")]
pub struct Glitch {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.3)]
    pub intensity: f32,
    #[field(min = 4.0, max = 64.0, speed = 1.0, default = 16.0)]
    pub block_size: f32,
    #[field(min = 0.0, max = 0.1, speed = 0.001, default = 0.01)]
    pub color_drift: f32,
    #[field(min = 0.1, max = 20.0, speed = 0.1, default = 5.0)]
    pub speed: f32,
    /// Seconds, advanced by [`sync_time`].
    #[field(skip, default = 0.0)]
    pub time: f32,
}

/// Drives the shader's clock.
///
/// Nothing was writing this before, which left the glitch stuck on one frame's
/// worth of displacement: the framework uploads the component's bytes verbatim
/// and interprets no field.
fn sync_time(mut q: Query<&mut Glitch>, time: Res<Time>) {
    for mut s in &mut q {
        s.time += time.delta_secs();
        if s.time > 1024.0 {
            s.time -= 1024.0;
        }
    }
}

#[derive(Default)]
pub struct GlitchPlugin;

impl Plugin for GlitchPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "glitch.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Glitch>::default());
        app.add_systems(Update, sync_time);
        app.register_inspectable::<Glitch>();
    }
}

renzora::plugin!(GlitchPlugin, Runtime);
