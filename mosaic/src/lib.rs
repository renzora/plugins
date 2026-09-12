//! Mosaic tile post-process effect.
//!
//! Unlike `pixelation`, the tiles keep a visible grout line and rounded corners,
//! so the result reads as tilework rather than as a low resolution.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `mosaic.wgsl`'s `MosaicSettings` must match field for field.
#[post_process(shader = "mosaic.wgsl", name = "Mosaic", icon = "grid-four")]
pub struct Mosaic {
    #[field(min = 4.0, max = 200.0, speed = 0.5, default = 40.0)]
    pub tile_size: f32,
    #[field(min = 0.0, max = 0.5, speed = 0.01, default = 0.05)]
    pub edge_thickness: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.3)]
    pub roundness: f32,
}

#[derive(Default)]
pub struct MosaicPlugin;

impl Plugin for MosaicPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "mosaic.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Mosaic>::default());
        app.register_inspectable::<Mosaic>();
    }
}

renzora::plugin!(MosaicPlugin, Runtime);
