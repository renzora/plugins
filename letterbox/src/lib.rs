//! Letterbox bars post-process effect.
//!
//! `aspect_ratio` at zero means "use `bar_height` directly". Give it a real
//! ratio instead and the bars size themselves to crop the view to it, which is
//! what you want for a cutscene that has to look the same on every display.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `letterbox.wgsl`'s `LetterboxSettings` must match field for field.
#[post_process(shader = "letterbox.wgsl", name = "Letterbox", icon = "rectangle")]
pub struct Letterbox {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.12)]
    pub bar_height: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.0)]
    pub softness: f32,
    #[field(min = 0.0, max = 3.0, speed = 0.01, default = 0.0)]
    pub aspect_ratio: f32,
}

#[derive(Default)]
pub struct LetterboxPlugin;

impl Plugin for LetterboxPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "letterbox.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Letterbox>::default());
        app.register_inspectable::<Letterbox>();
    }
}

renzora::plugin!(LetterboxPlugin, Runtime);
