//! Invert post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `invert.wgsl`'s `InvertSettings` must match field for field.
#[post_process(shader = "invert.wgsl", name = "Invert", icon = "circles-three")]
pub struct Invert {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 1.0)]
    pub intensity: f32,
}

#[derive(Default)]
pub struct InvertPlugin;

impl Plugin for InvertPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "invert.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Invert>::default());
        app.register_inspectable::<Invert>();
    }
}

renzora::plugin!(InvertPlugin, Runtime);
