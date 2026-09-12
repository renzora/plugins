//! Screen-space fog overlay post-process effect.
//!
//! A cheap height-fade over the finished picture, not a depth-aware fog. For fog
//! that respects distance, use the engine's own Distance Fog on the World
//! Environment instead.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `fog_overlay.wgsl`'s `FogOverlaySettings` must match field for field.
#[post_process(shader = "fog_overlay.wgsl", name = "Fog Overlay", icon = "cloud-fog")]
pub struct FogOverlay {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.3)]
    pub density: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.3)]
    pub height: f32,
    #[field(skip, default = 0.7)]
    pub color_r: f32,
    #[field(skip, default = 0.75)]
    pub color_g: f32,
    #[field(skip, default = 0.8)]
    pub color_b: f32,
}

#[derive(Default)]
pub struct FogOverlayPlugin;

impl Plugin for FogOverlayPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "fog_overlay.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<FogOverlay>::default());
        app.register_inspectable::<FogOverlay>();
    }
}

renzora::plugin!(FogOverlayPlugin, Runtime);
