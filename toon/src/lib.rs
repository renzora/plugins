//! Cel-shading post-process effect.
//!
//! Quantises luminance into bands and draws an edge over the result, so it is
//! `posterize` and `outline` in one pass rather than two, which keeps the edge
//! aligned to the bands it is outlining.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `toon.wgsl`'s `ToonSettings` must match field for field.
#[post_process(shader = "toon.wgsl", name = "Toon", icon = "paint-brush")]
pub struct Toon {
    #[field(min = 2.0, max = 16.0, speed = 0.1, default = 4.0)]
    pub levels: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.005, default = 0.1)]
    pub edge_threshold: f32,
    #[field(min = 0.5, max = 5.0, speed = 0.05, default = 1.0)]
    pub edge_thickness: f32,
    #[field(min = 0.0, max = 3.0, speed = 0.02, default = 1.2)]
    pub saturation_boost: f32,
}

#[derive(Default)]
pub struct ToonPlugin;

impl Plugin for ToonPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "toon.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Toon>::default());
        app.register_inspectable::<Toon>();
    }
}

renzora::plugin!(ToonPlugin, Runtime);
