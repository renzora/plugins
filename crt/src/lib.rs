//! CRT monitor post-process effect.
//!
//! Screen curvature, scanlines, a colour fringe and a vignette in one pass. Each
//! exists separately in this set, but a CRT wants them sharing a curved UV: the
//! scanlines have to bend with the glass, and stacking four passes would leave
//! them flat over a curved picture.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `crt.wgsl`'s `CrtSettings` must match field for field.
#[post_process(shader = "crt.wgsl", name = "CRT", icon = "television-simple")]
pub struct Crt {
    #[field(min = 0.0, max = 2.0, speed = 0.01, default = 0.3)]
    pub scanline_intensity: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.02)]
    pub curvature: f32,
    #[field(min = 0.0, max = 0.1, speed = 0.001, default = 0.003)]
    pub chromatic_amount: f32,
    #[field(min = 0.0, max = 2.0, speed = 0.01, default = 0.5)]
    pub vignette_amount: f32,
}

#[derive(Default)]
pub struct CrtPlugin;

impl Plugin for CrtPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "crt.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Crt>::default());
        app.register_inspectable::<Crt>();
    }
}

renzora::plugin!(CrtPlugin, Runtime);
