//! Directional RGB split post-process effect.
//!
//! Red and blue are pushed opposite ways along one axis and green is left where
//! it is, which is why only two offsets are authored. `angle` is in radians, so
//! its maximum is a full turn.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `color_split.wgsl`'s `ColorSplitSettings` must match field for field.
#[post_process(shader = "color_split.wgsl", name = "Color Split", icon = "arrows-out-line-horizontal")]
pub struct ColorSplit {
    #[field(min = 0.0, max = 0.05, speed = 0.001, default = 0.005)]
    pub offset_r: f32,
    #[field(min = 0.0, max = 0.05, speed = 0.001, default = 0.005)]
    pub offset_b: f32,
    #[field(min = 0.0, max = 6.283, speed = 0.01, default = 0.0)]
    pub angle: f32,
}

#[derive(Default)]
pub struct ColorSplitPlugin;

impl Plugin for ColorSplitPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "color_split.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<ColorSplit>::default());
        app.register_inspectable::<ColorSplit>();
    }
}

renzora::plugin!(ColorSplitPlugin, Runtime);
