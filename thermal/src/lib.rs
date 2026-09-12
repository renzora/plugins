//! Thermal vision post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `thermal.wgsl`'s `ThermalSettings` must match field for field.
#[post_process(shader = "thermal.wgsl", name = "Thermal Vision", icon = "thermometer")]
pub struct Thermal {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 1.0)]
    pub intensity: f32,
    #[field(min = 0.1, max = 3.0, speed = 0.01, default = 1.5)]
    pub contrast: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.3)]
    pub cold_threshold: f32,
}

#[derive(Default)]
pub struct ThermalPlugin;

impl Plugin for ThermalPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "thermal.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Thermal>::default());
        app.register_inspectable::<Thermal>();
    }
}

renzora::plugin!(ThermalPlugin, Runtime);
