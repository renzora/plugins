//! Threshold post-process effect.
//!
//! `smoothness` is what keeps the cut from aliasing: at zero the split is a hard
//! step and every edge in the picture stairsteps, so the default eases it over a
//! narrow band either side of the threshold.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `threshold.wgsl`'s `ThresholdSettings` must match field for field.
#[post_process(shader = "threshold.wgsl", name = "Threshold", icon = "circle-half-tilt")]
pub struct Threshold {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub threshold: f32,
    #[field(min = 0.0, max = 0.5, speed = 0.01, default = 0.05)]
    pub smoothness: f32,
}

#[derive(Default)]
pub struct ThresholdPlugin;

impl Plugin for ThresholdPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "threshold.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Threshold>::default());
        app.register_inspectable::<Threshold>();
    }
}

renzora::plugin!(ThresholdPlugin, Runtime);
