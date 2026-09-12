//! Radial blur post-process effect.
//!
//! Unlike `swirl` and `kaleidoscope`, the centre is authored: a radial blur is
//! usually aimed at something, so the point it streaks away from is the whole
//! control.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `radial_blur.wgsl`'s `RadialBlurSettings` must match field for field.
#[post_process(shader = "radial_blur.wgsl", name = "Radial Blur", icon = "sun-dim")]
pub struct RadialBlur {
    #[field(min = 0.0, max = 0.2, speed = 0.001, default = 0.02)]
    pub intensity: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub center_x: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub center_y: f32,
    #[field(min = 4.0, max = 32.0, speed = 1.0, default = 8.0)]
    pub samples: f32,
}

#[derive(Default)]
pub struct RadialBlurPlugin;

impl Plugin for RadialBlurPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "radial_blur.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<RadialBlur>::default());
        app.register_inspectable::<RadialBlur>();
    }
}

renzora::plugin!(RadialBlurPlugin, Runtime);
