//! Screen-space outline post-process effect.
//!
//! `mix_mode` crossfades between drawing the outline over the picture and
//! replacing the picture with it, so one effect covers both the comic-book look
//! and a plain line render.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `outline.wgsl`'s `OutlineSettings` must match field for field.
#[post_process(shader = "outline.wgsl", name = "Outline", icon = "bounding-box")]
pub struct Outline {
    #[field(min = 0.5, max = 5.0, speed = 0.05, default = 1.0)]
    pub thickness: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.005, default = 0.1)]
    pub threshold: f32,
    #[field(skip, default = 0.0)]
    pub color_r: f32,
    #[field(skip, default = 0.0)]
    pub color_g: f32,
    #[field(skip, default = 0.0)]
    pub color_b: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.0)]
    pub mix_mode: f32,
}

#[derive(Default)]
pub struct OutlinePlugin;

impl Plugin for OutlinePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "outline.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Outline>::default());
        app.register_inspectable::<Outline>();
    }
}

renzora::plugin!(OutlinePlugin, Runtime);
