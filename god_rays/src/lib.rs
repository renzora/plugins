//! Volumetric light shaft post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `god_rays.wgsl`'s `GodRaysSettings` must match field for field.
#[post_process(shader = "god_rays.wgsl", name = "God Rays", icon = "sun-horizon")]
pub struct GodRays {
    #[field(min = 0.0, max = 2.0, speed = 0.01, default = 0.5)]
    pub intensity: f32,
    #[field(min = 0.9, max = 1.0, speed = 0.001, default = 0.97)]
    pub decay: f32,
    #[field(min = 0.0, max = 2.0, speed = 0.01, default = 1.0)]
    pub density: f32,
    /// Ray-march step count. It MUST sit here, between `density` and
    /// `light_pos_x`, because that is where the uniform block has it. Editable
    /// now that the inspector has an integer field type; under the C ABI it had
    /// to be skipped, because there was none. The shader clamps to 128.
    #[field(min = 8.0, max = 128.0, default = 64.0)]
    pub num_samples: u32,
    #[field(min = -1.0, max = 2.0, speed = 0.01, default = 0.5)]
    pub light_pos_x: f32,
    #[field(min = -1.0, max = 2.0, speed = 0.01, default = 0.3)]
    pub light_pos_y: f32,
}

#[derive(Default)]
pub struct GodRaysPlugin;

impl Plugin for GodRaysPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "god_rays.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<GodRays>::default());
        app.register_inspectable::<GodRays>();
    }
}

renzora::plugin!(GodRaysPlugin, Runtime);
