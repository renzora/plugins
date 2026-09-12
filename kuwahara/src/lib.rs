//! Kuwahara edge-preserving smoothing post-process effect.
//!
//! Flattens flat areas while leaving edges sharp, which is the painterly look
//! without the brush clumps `oil_painting` adds.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `kuwahara.wgsl`'s `KuwaharaSettings` must match field for field.
#[post_process(shader = "kuwahara.wgsl", name = "Kuwahara", icon = "paint-roller")]
pub struct Kuwahara {
    #[field(min = 1.0, max = 8.0, speed = 0.1, default = 3.0)]
    pub radius: f32,
}

#[derive(Default)]
pub struct KuwaharaPlugin;

impl Plugin for KuwaharaPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "kuwahara.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Kuwahara>::default());
        app.register_inspectable::<Kuwahara>();
    }
}

renzora::plugin!(KuwaharaPlugin, Runtime);
