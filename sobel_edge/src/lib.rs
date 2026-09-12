//! Sobel edge-detection post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `sobel_edge.wgsl`'s `SobelEdgeSettings` must match field for field.
#[post_process(shader = "sobel_edge.wgsl", name = "Sobel Edge", icon = "polygon")]
pub struct SobelEdge {
    #[field(min = 0.0, max = 5.0, speed = 0.01, default = 1.0)]
    pub intensity: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.1)]
    pub threshold: f32,
    #[field(skip, default = 0.0)]
    pub color_r: f32,
    #[field(skip, default = 1.0)]
    pub color_g: f32,
    #[field(skip, default = 0.0)]
    pub color_b: f32,
}

#[derive(Default)]
pub struct SobelEdgePlugin;

impl Plugin for SobelEdgePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "sobel_edge.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<SobelEdge>::default());
        app.register_inspectable::<SobelEdge>();
    }
}

renzora::plugin!(SobelEdgePlugin, Runtime);
