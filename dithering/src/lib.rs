//! Ordered (Bayer) dithering post-process effect.
//!
//! The threshold matrix is generated in the shader, so the effect ships no
//! texture and the pattern stays pixel-exact at any render resolution.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `dithering.wgsl`'s `DitheringSettings` must match field for field.
#[post_process(shader = "dithering.wgsl", name = "Dithering", icon = "dots-nine")]
pub struct Dithering {
    #[field(min = 2.0, max = 32.0, speed = 0.5, default = 8.0)]
    pub color_depth: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 1.0)]
    pub intensity: f32,
}

#[derive(Default)]
pub struct DitheringPlugin;

impl Plugin for DitheringPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "dithering.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Dithering>::default());
        app.register_inspectable::<Dithering>();
    }
}

renzora::plugin!(DitheringPlugin, Runtime);
