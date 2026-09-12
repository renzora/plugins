//! Glowing edge-detection post-process effect.
//!
//! The glow colour stays three skipped floats rather than becoming a `Vec3`
//! colour field: the shader reads them at those exact offsets, and widening the
//! type would move every field after it.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `edge_glow.wgsl`'s `EdgeGlowSettings` must match field for field.
#[post_process(shader = "edge_glow.wgsl", name = "Edge Glow", icon = "line-segments")]
pub struct EdgeGlow {
    #[field(min = 0.0, max = 1.0, speed = 0.005, default = 0.1)]
    pub threshold: f32,
    #[field(min = 0.0, max = 5.0, speed = 0.05, default = 2.0)]
    pub glow_intensity: f32,
    #[field(skip, default = 0.0)]
    pub color_r: f32,
    #[field(skip, default = 1.0)]
    pub color_g: f32,
    #[field(skip, default = 1.0)]
    pub color_b: f32,
}

#[derive(Default)]
pub struct EdgeGlowPlugin;

impl Plugin for EdgeGlowPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "edge_glow.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<EdgeGlow>::default());
        app.register_inspectable::<EdgeGlow>();
    }
}

renzora::plugin!(EdgeGlowPlugin, Runtime);
