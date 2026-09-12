//! Cross-hatch shading post-process effect.
//!
//! `angle` is in radians and tops out at a quarter turn: past that the hatching
//! is the same set of lines again, mirrored.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `cross_hatch.wgsl`'s `CrossHatchSettings` must match field for field.
#[post_process(shader = "cross_hatch.wgsl", name = "Cross Hatch", icon = "scribble")]
pub struct CrossHatch {
    #[field(min = 2.0, max = 100.0, speed = 0.5, default = 30.0)]
    pub density: f32,
    #[field(min = 0.01, max = 0.5, speed = 0.01, default = 0.1)]
    pub thickness: f32,
    #[field(min = 0.0, max = 1.57, speed = 0.01, default = 0.785)]
    pub angle: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.9)]
    pub brightness: f32,
}

#[derive(Default)]
pub struct CrossHatchPlugin;

impl Plugin for CrossHatchPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "cross_hatch.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<CrossHatch>::default());
        app.register_inspectable::<CrossHatch>();
    }
}

renzora::plugin!(CrossHatchPlugin, Runtime);
