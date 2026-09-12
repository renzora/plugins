//! Vibrance post-process effect.
//!
//! Saturation weighted by how unsaturated a pixel already is, so it lifts muted
//! colour without pushing already-vivid pixels further. `intensity` goes
//! negative, which desaturates the same way round.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `vibrance.wgsl`'s `VibranceSettings` must match field for field.
#[post_process(shader = "vibrance.wgsl", name = "Vibrance", icon = "drop")]
pub struct Vibrance {
    #[field(min = -1.0, max = 2.0, speed = 0.01, default = 0.5)]
    pub intensity: f32,
}

#[derive(Default)]
pub struct VibrancePlugin;

impl Plugin for VibrancePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "vibrance.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Vibrance>::default());
        app.register_inspectable::<Vibrance>();
    }
}

renzora::plugin!(VibrancePlugin, Runtime);
