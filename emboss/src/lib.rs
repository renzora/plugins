//! Emboss post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `emboss.wgsl`'s `EmbossSettings` must match field for field.
#[post_process(shader = "emboss.wgsl", name = "Emboss", icon = "stamp")]
pub struct Emboss {
    #[field(min = 0.0, max = 3.0, speed = 0.01, default = 1.0)]
    pub strength: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub mix_amount: f32,
}

#[derive(Default)]
pub struct EmbossPlugin;

impl Plugin for EmbossPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "emboss.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Emboss>::default());
        app.register_inspectable::<Emboss>();
    }
}

renzora::plugin!(EmbossPlugin, Runtime);
