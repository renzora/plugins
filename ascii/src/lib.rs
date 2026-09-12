//! ASCII art post-process effect.
//!
//! The character shapes are generated in the shader from luminance rather than
//! sampled from a font atlas, so there is no texture to ship and the cell size
//! is free to be any number rather than a multiple of a glyph.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `ascii.wgsl`'s `AsciiSettings` must match field for field.
#[post_process(shader = "ascii.wgsl", name = "ASCII", icon = "text-aa")]
pub struct Ascii {
    #[field(min = 2.0, max = 32.0, speed = 0.5, default = 8.0)]
    pub char_size: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub color_mix: f32,
    #[field(min = 0.5, max = 3.0, speed = 0.01, default = 1.2)]
    pub contrast: f32,
}

#[derive(Default)]
pub struct AsciiPlugin;

impl Plugin for AsciiPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "ascii.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Ascii>::default());
        app.register_inspectable::<Ascii>();
    }
}

renzora::plugin!(AsciiPlugin, Runtime);
