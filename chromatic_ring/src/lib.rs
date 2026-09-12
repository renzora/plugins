//! Radial chromatic aberration post-process effect.
//!
//! The radial counterpart to `chromatic_aberration`: the split grows with
//! distance from the centre rather than running one way across the screen, which
//! is what a real lens does. `radius` is where it starts and `falloff` how
//! quickly it climbs from there.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `chromatic_ring.wgsl`'s `ChromaticRingSettings` must match field for field.
#[post_process(shader = "chromatic_ring.wgsl", name = "Chromatic Ring", icon = "circle-dashed")]
pub struct ChromaticRing {
    #[field(min = 0.0, max = 0.05, speed = 0.001, default = 0.008)]
    pub intensity: f32,
    #[field(min = 0.0, max = 2.0, speed = 0.01, default = 0.8)]
    pub radius: f32,
    #[field(min = 0.01, max = 1.0, speed = 0.01, default = 0.4)]
    pub falloff: f32,
}

#[derive(Default)]
pub struct ChromaticRingPlugin;

impl Plugin for ChromaticRingPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "chromatic_ring.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<ChromaticRing>::default());
        app.register_inspectable::<ChromaticRing>();
    }
}

renzora::plugin!(ChromaticRingPlugin, Runtime);
