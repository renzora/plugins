//! Halftone dot-screen post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `halftone.wgsl`'s `HalftoneSettings` must match field for field.
#[post_process(shader = "halftone.wgsl", name = "Halftone", icon = "dots-six")]
pub struct Halftone {
    #[field(min = 2.0, max = 20.0, speed = 0.1, default = 4.0)]
    pub dot_size: f32,
    #[field(min = 0.0, max = 3.14159, speed = 0.01, default = 0.785)]
    pub angle: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 1.0)]
    pub intensity: f32,
}

#[derive(Default)]
pub struct HalftonePlugin;

impl Plugin for HalftonePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "halftone.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Halftone>::default());
        app.register_inspectable::<Halftone>();
    }
}

renzora::plugin!(HalftonePlugin, Runtime);
