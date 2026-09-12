//! Dreamy soft-glow post-process effect.
//!
//! A thresholded blur mixed back over the picture. Unlike bloom it keeps the
//! blurred copy visible in the midtones rather than only where the image is
//! bright, which is what gives it the hazy look rather than a highlight sheen.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `dream.wgsl`'s `DreamSettings` must match field for field.
#[post_process(shader = "dream.wgsl", name = "Dream", icon = "cloud")]
pub struct Dream {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.4)]
    pub intensity: f32,
    #[field(min = 1.0, max = 10.0, speed = 0.1, default = 3.0)]
    pub blur_radius: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub threshold: f32,
}

#[derive(Default)]
pub struct DreamPlugin;

impl Plugin for DreamPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "dream.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Dream>::default());
        app.register_inspectable::<Dream>();
    }
}

renzora::plugin!(DreamPlugin, Runtime);
