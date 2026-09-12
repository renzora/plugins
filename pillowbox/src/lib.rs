//! Pillarbox bars post-process effect.
//!
//! The vertical counterpart to `letterbox`, for a narrow picture on a wide
//! display. `aspect_ratio` at zero means "use `bar_width` directly".

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `pillowbox.wgsl`'s `PillowboxSettings` must match field for field.
#[post_process(shader = "pillowbox.wgsl", name = "Pillarbox", icon = "square")]
pub struct Pillowbox {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.15)]
    pub bar_width: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.0)]
    pub softness: f32,
    #[field(min = 0.0, max = 3.0, speed = 0.01, default = 0.0)]
    pub aspect_ratio: f32,
}

#[derive(Default)]
pub struct PillowboxPlugin;

impl Plugin for PillowboxPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "pillowbox.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Pillowbox>::default());
        app.register_inspectable::<Pillowbox>();
    }
}

renzora::plugin!(PillowboxPlugin, Runtime);
