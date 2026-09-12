//! Falling-code rain post-process effect.
//!
//! Seven user fields plus `enabled` fills two `vec4`s exactly, so this is the
//! one effect in the set that needs no padding at all.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled`, so `matrix.wgsl`'s `MatrixSettings` must match
/// field for field.
#[post_process(shader = "matrix.wgsl", name = "Matrix Rain", icon = "binary")]
pub struct Matrix {
    #[field(min = 0.1, max = 10.0, speed = 0.05, default = 2.0)]
    pub speed: f32,
    #[field(min = 5.0, max = 50.0, speed = 0.5, default = 20.0)]
    pub density: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub glow: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.8)]
    pub trail_length: f32,
    #[field(skip, default = 0.0)]
    pub color_r: f32,
    #[field(skip, default = 1.0)]
    pub color_g: f32,
    /// Seconds, advanced by [`sync_time`].
    #[field(skip, default = 0.0)]
    pub time: f32,
}

/// Drives the shader's clock.
///
/// Nothing was writing this before, which left the rain hanging motionless: the
/// framework uploads the component's bytes verbatim and interprets no field, so
/// an animated effect has to tick its own.
fn sync_time(mut q: Query<&mut Matrix>, time: Res<Time>) {
    for mut s in &mut q {
        s.time += time.delta_secs();
        if s.time > 1024.0 {
            s.time -= 1024.0;
        }
    }
}

#[derive(Default)]
pub struct MatrixPlugin;

impl Plugin for MatrixPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "matrix.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Matrix>::default());
        app.add_systems(Update, sync_time);
        app.register_inspectable::<Matrix>();
    }
}

renzora::plugin!(MatrixPlugin, Runtime);
