//! Colour grading post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `color_grading.wgsl`'s `ColorGradingSettings` must match field for field.
#[post_process(shader = "color_grading.wgsl", name = "Color Grading", icon = "sliders")]
pub struct ColorGrading {
    #[field(min = 0.0, max = 3.0, speed = 0.01, default = 1.0)]
    pub brightness: f32,
    #[field(min = 0.0, max = 3.0, speed = 0.01, default = 1.0)]
    pub contrast: f32,
    #[field(min = 0.0, max = 3.0, speed = 0.01, default = 1.0)]
    pub saturation: f32,
    #[field(min = 0.1, max = 3.0, speed = 0.01, default = 1.0)]
    pub gamma: f32,
    #[field(min = -1.0, max = 1.0, speed = 0.01, default = 0.0)]
    pub temperature: f32,
    #[field(min = -1.0, max = 1.0, speed = 0.01, default = 0.0)]
    pub tint: f32,
}

#[derive(Default)]
pub struct ColorGradingPlugin;

impl Plugin for ColorGradingPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "color_grading.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<ColorGrading>::default());
        app.register_inspectable::<ColorGrading>();
    }
}

renzora::plugin!(ColorGradingPlugin, Runtime);
