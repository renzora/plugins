//! Chromatic aberration post-process effect.
//!
//! The direction is a fixed horizontal split rather than an inspector field:
//! a directional aberration reads as a camera fault, and the one people reach
//! for is lateral. `chromatic_ring` is the radial version.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `chromatic_aberration.wgsl`'s `ChromaticAberrationSettings` must match field
/// for field.
#[post_process(
    shader = "chromatic_aberration.wgsl",
    name = "Chromatic Aberration",
    icon = "rainbow"
)]
pub struct ChromaticAberration {
    #[field(min = 0.0, max = 0.1, speed = 0.001, default = 0.005)]
    pub intensity: f32,
    #[field(min = 1.0, max = 16.0, speed = 1.0, default = 3.0)]
    pub samples: f32,
    #[field(skip, default = 1.0)]
    pub direction_x: f32,
    #[field(skip, default = 0.0)]
    pub direction_y: f32,
}

#[derive(Default)]
pub struct ChromaticAberrationPlugin;

impl Plugin for ChromaticAberrationPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "chromatic_aberration.wgsl");
        app.add_plugins(
            renzora::postprocess::PostProcessPlugin::<ChromaticAberration>::default(),
        );
        app.register_inspectable::<ChromaticAberration>();
    }
}

renzora::plugin!(ChromaticAberrationPlugin, Runtime);
