//! Oil painting post-process effect.
//!
//! A simplified Kuwahara filter with luminance bucketing on top, which is what
//! gives it visible brush clumps rather than the smooth flattening `kuwahara`
//! produces. The sample loop is O(radius²), so `radius` is the cost knob.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `oil_painting.wgsl`'s `OilPaintingSettings` must match field for field.
#[post_process(shader = "oil_painting.wgsl", name = "Oil Painting", icon = "paint-bucket")]
pub struct OilPainting {
    #[field(min = 1.0, max = 8.0, speed = 0.1, default = 3.0)]
    pub radius: f32,
    #[field(min = 4.0, max = 32.0, speed = 0.5, default = 8.0)]
    pub levels: f32,
}

#[derive(Default)]
pub struct OilPaintingPlugin;

impl Plugin for OilPaintingPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "oil_painting.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<OilPainting>::default());
        app.register_inspectable::<OilPainting>();
    }
}

renzora::plugin!(OilPaintingPlugin, Runtime);
