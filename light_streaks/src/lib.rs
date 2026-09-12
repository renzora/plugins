//! Anamorphic light streak post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `light_streaks.wgsl`'s `LightStreaksSettings` must match field for field.
#[post_process(shader = "light_streaks.wgsl", name = "Light Streaks", icon = "shooting-star")]
pub struct LightStreaks {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.4)]
    pub intensity: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.7)]
    pub threshold: f32,
    #[field(min = 4.0, max = 32.0, speed = 1.0, default = 12.0)]
    pub samples: f32,
    #[field(min = 0.0, max = 6.283, speed = 0.01, default = 0.0)]
    pub direction: f32,
}

#[derive(Default)]
pub struct LightStreaksPlugin;

impl Plugin for LightStreaksPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "light_streaks.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<LightStreaks>::default());
        app.register_inspectable::<LightStreaks>();
    }
}

renzora::plugin!(LightStreaksPlugin, Runtime);
