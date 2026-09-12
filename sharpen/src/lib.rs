//! Sharpen post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `sharpen.wgsl`'s `SharpenSettings` must match field for field.
#[post_process(shader = "sharpen.wgsl", name = "Sharpen", icon = "triangle")]
pub struct Sharpen {
    #[field(min = 0.0, max = 3.0, speed = 0.01, default = 0.5)]
    pub strength: f32,
}

#[derive(Default)]
pub struct SharpenPlugin;

impl Plugin for SharpenPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "sharpen.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Sharpen>::default());
        app.register_inspectable::<Sharpen>();
    }
}

renzora::plugin!(SharpenPlugin, Runtime);
