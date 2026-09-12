//! Tilt-shift depth-of-field post-process effect.
//!
//! A screen-space fake rather than a real depth-of-field: the focus band is a
//! horizontal strip at `focus_position`, so it costs nothing to sample the depth
//! buffer and works on a 2D scene that has none.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `tilt_shift.wgsl`'s `TiltShiftSettings` must match field for field.
#[post_process(shader = "tilt_shift.wgsl", name = "Tilt Shift", icon = "camera")]
pub struct TiltShift {
    #[field(min = 0.0, max = 10.0, speed = 0.1, default = 3.0)]
    pub blur_amount: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub focus_position: f32,
    #[field(min = 0.01, max = 0.5, speed = 0.01, default = 0.1)]
    pub focus_width: f32,
    #[field(min = 0.01, max = 0.5, speed = 0.01, default = 0.15)]
    pub focus_falloff: f32,
}

#[derive(Default)]
pub struct TiltShiftPlugin;

impl Plugin for TiltShiftPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "tilt_shift.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<TiltShift>::default());
        app.register_inspectable::<TiltShift>();
    }
}

renzora::plugin!(TiltShiftPlugin, Runtime);
