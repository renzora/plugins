//! Hexagonal pixelation post-process effect.
//!
//! The hex grid is computed in axial coordinates in the shader, so `hex_size` is
//! a pixel width rather than a cell count and the pattern stays the same size
//! whatever the render resolution.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `hex_pixelate.wgsl`'s `HexPixelateSettings` must match field for field.
#[post_process(shader = "hex_pixelate.wgsl", name = "Hex Pixelate", icon = "hexagon")]
pub struct HexPixelate {
    #[field(min = 2.0, max = 50.0, speed = 0.5, default = 10.0)]
    pub hex_size: f32,
}

#[derive(Default)]
pub struct HexPixelatePlugin;

impl Plugin for HexPixelatePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "hex_pixelate.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<HexPixelate>::default());
        app.register_inspectable::<HexPixelate>();
    }
}

renzora::plugin!(HexPixelatePlugin, Runtime);
