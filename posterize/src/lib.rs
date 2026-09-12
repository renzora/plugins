//! Posterize post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `posterize.wgsl`'s `PosterizeSettings` must match field for field.
#[post_process(shader = "posterize.wgsl", name = "Posterize", icon = "stack-simple")]
pub struct Posterize {
    #[field(min = 2.0, max = 64.0, speed = 1.0, default = 8.0)]
    pub levels: f32,
}

#[derive(Default)]
pub struct PosterizePlugin;

impl Plugin for PosterizePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "posterize.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Posterize>::default());
        app.register_inspectable::<Posterize>();
    }
}

renzora::plugin!(PosterizePlugin, Runtime);
