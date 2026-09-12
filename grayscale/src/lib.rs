//! Grayscale post-process effect.
//!
//! The luminance weights are real values the shader reads every pixel, but they
//! are the Rec. 709 constants rather than something to put three sliders on, so
//! `#[field(skip)]` keeps them in the struct and out of the inspector.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `grayscale.wgsl`'s `GrayscaleSettings` must match field for field.
#[post_process(shader = "grayscale.wgsl", name = "Grayscale", icon = "circle-half")]
pub struct Grayscale {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 1.0)]
    pub intensity: f32,
    #[field(skip, default = 0.2126)]
    pub luminance_r: f32,
    #[field(skip, default = 0.7152)]
    pub luminance_g: f32,
    #[field(skip, default = 0.0722)]
    pub luminance_b: f32,
}

#[derive(Default)]
pub struct GrayscalePlugin;

impl Plugin for GrayscalePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "grayscale.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Grayscale>::default());
        app.register_inspectable::<Grayscale>();
    }
}

renzora::plugin!(GrayscalePlugin, Runtime);
