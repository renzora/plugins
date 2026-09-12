//! Palette quantization post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `palette_quantization.wgsl`'s `PaletteQuantizationSettings` must match field
/// for field.
#[post_process(
    shader = "palette_quantization.wgsl",
    name = "Palette Quantization",
    icon = "palette"
)]
pub struct PaletteQuantization {
    /// Quantization levels per channel. It MUST come first, because that is
    /// where the uniform block has it. Editable now that the inspector has an
    /// integer field type; under the C ABI it had to be skipped, because there
    /// was none. 8 levels is 512 colours, the classic retro-palette look this
    /// effect is for, and the shader floors it at 2.
    #[field(min = 2.0, max = 64.0, default = 8.0)]
    pub num_colors: u32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub dithering: f32,
}

#[derive(Default)]
pub struct PaletteQuantizationPlugin;

impl Plugin for PaletteQuantizationPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "palette_quantization.wgsl");
        app.add_plugins(
            renzora::postprocess::PostProcessPlugin::<PaletteQuantization>::default(),
        );
        app.register_inspectable::<PaletteQuantization>();
    }
}

renzora::plugin!(PaletteQuantizationPlugin, Runtime);
