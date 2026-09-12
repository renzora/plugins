//! Pencil sketch post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `sketch.wgsl`'s `SketchSettings` must match field for field.
#[post_process(shader = "sketch.wgsl", name = "Sketch", icon = "pencil")]
pub struct Sketch {
    #[field(min = 0.0, max = 5.0, speed = 0.01, default = 1.5)]
    pub edge_strength: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.95)]
    pub paper_brightness: f32,
    #[field(min = 0.5, max = 5.0, speed = 0.1, default = 1.0)]
    pub line_density: f32,
}

#[derive(Default)]
pub struct SketchPlugin;

impl Plugin for SketchPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "sketch.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Sketch>::default());
        app.register_inspectable::<Sketch>();
    }
}

renzora::plugin!(SketchPlugin, Runtime);
