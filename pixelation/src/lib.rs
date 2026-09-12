//! Pixelation post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `pixelation.wgsl`'s `PixelationSettings` must match field for field.
#[post_process(shader = "pixelation.wgsl", name = "Pixelation", icon = "grid-nine")]
pub struct Pixelation {
    #[field(min = 1.0, max = 64.0, speed = 0.5, default = 4.0)]
    pub pixel_size: f32,
}

#[derive(Default)]
pub struct PixelationPlugin;

impl Plugin for PixelationPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "pixelation.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Pixelation>::default());
        app.register_inspectable::<Pixelation>();
    }
}

renzora::plugin!(PixelationPlugin, Runtime);
