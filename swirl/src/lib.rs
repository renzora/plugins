//! Swirl distortion post-process effect.
//!
//! The centre stays two skipped floats fixed at the middle of the screen: the
//! shader reads them at those exact offsets, and a `Vec2` field would move
//! everything after it in the uniform.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `swirl.wgsl`'s `SwirlSettings` must match field for field.
#[post_process(shader = "swirl.wgsl", name = "Swirl", icon = "spiral")]
pub struct Swirl {
    #[field(min = -10.0, max = 10.0, speed = 0.01, default = 3.0)]
    pub angle: f32,
    #[field(min = 0.01, max = 2.0, speed = 0.01, default = 0.5)]
    pub radius: f32,
    #[field(skip, default = 0.5)]
    pub center_x: f32,
    #[field(skip, default = 0.5)]
    pub center_y: f32,
}

#[derive(Default)]
pub struct SwirlPlugin;

impl Plugin for SwirlPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "swirl.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Swirl>::default());
        app.register_inspectable::<Swirl>();
    }
}

renzora::plugin!(SwirlPlugin, Runtime);
