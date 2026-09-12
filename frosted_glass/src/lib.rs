//! Frosted glass post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `frosted_glass.wgsl`'s `FrostedGlassSettings` must match field for field.
#[post_process(shader = "frosted_glass.wgsl", name = "Frosted Glass", icon = "snowflake")]
pub struct FrostedGlass {
    #[field(min = 0.0, max = 0.05, speed = 0.001, default = 0.01)]
    pub intensity: f32,
    #[field(min = 1.0, max = 50.0, speed = 0.5, default = 10.0)]
    pub scale: f32,
}

#[derive(Default)]
pub struct FrostedGlassPlugin;

impl Plugin for FrostedGlassPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "frosted_glass.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<FrostedGlass>::default());
        app.register_inspectable::<FrostedGlass>();
    }
}

renzora::plugin!(FrostedGlassPlugin, Runtime);
