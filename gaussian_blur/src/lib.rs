//! Gaussian blur post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `gaussian_blur.wgsl`'s `GaussianBlurSettings` must match field for field.
#[post_process(shader = "gaussian_blur.wgsl", name = "Gaussian Blur", icon = "drop-half-bottom")]
pub struct GaussianBlur {
    #[field(min = 0.1, max = 20.0, speed = 0.1, default = 2.0)]
    pub sigma: f32,
    /// Tap count per axis. Editable now that the inspector has an integer field
    /// type; under the C ABI it had to be skipped, because there was none. The
    /// shader clamps to 15 and the loop is O(n²), so 9 is the quality/cost knee
    /// rather than the ceiling.
    #[field(min = 1.0, max = 15.0, default = 9.0)]
    pub kernel_size: u32,
}

#[derive(Default)]
pub struct GaussianBlurPlugin;

impl Plugin for GaussianBlurPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "gaussian_blur.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<GaussianBlur>::default());
        app.register_inspectable::<GaussianBlur>();
    }
}

renzora::plugin!(GaussianBlurPlugin, Runtime);
