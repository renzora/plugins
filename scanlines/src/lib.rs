//! CRT scanline post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `scanlines.wgsl`'s `ScanlinesSettings` must match field for field.
#[post_process(shader = "scanlines.wgsl", name = "Scanlines", icon = "rows")]
pub struct Scanlines {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.15)]
    pub intensity: f32,
    #[field(min = 10.0, max = 2000.0, speed = 10.0, default = 800.0)]
    pub count: f32,
    #[field(min = 0.0, max = 10.0, speed = 0.1, default = 0.0)]
    pub speed: f32,
}

#[derive(Default)]
pub struct ScanlinesPlugin;

impl Plugin for ScanlinesPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "scanlines.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Scanlines>::default());
        app.register_inspectable::<Scanlines>();
    }
}

renzora::plugin!(ScanlinesPlugin, Runtime);
