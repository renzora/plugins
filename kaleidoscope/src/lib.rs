//! Kaleidoscope mirror post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `kaleidoscope.wgsl`'s `KaleidoscopeSettings` must match field for field.
#[post_process(shader = "kaleidoscope.wgsl", name = "Kaleidoscope", icon = "flower-lotus")]
pub struct Kaleidoscope {
    #[field(min = 2.0, max = 32.0, speed = 0.1, default = 6.0)]
    pub segments: f32,
    #[field(min = 0.0, max = 6.283, speed = 0.01, default = 0.0)]
    pub rotation: f32,
    #[field(skip, default = 0.5)]
    pub center_x: f32,
    #[field(skip, default = 0.5)]
    pub center_y: f32,
}

#[derive(Default)]
pub struct KaleidoscopePlugin;

impl Plugin for KaleidoscopePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "kaleidoscope.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Kaleidoscope>::default());
        app.register_inspectable::<Kaleidoscope>();
    }
}

renzora::plugin!(KaleidoscopePlugin, Runtime);
