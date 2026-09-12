//! Sepia tone mapping, as a native plugin.
//!
//! The simplest of the effects, and the one that shows `#[field(skip)]`: the
//! tone weights are real values the shader reads every pixel, but they are a
//! tuned constant rather than something to put three sliders on. Skipping keeps
//! them in the struct and out of the inspector.
//!
//! `#[post_process]` writes everything else — the derives, the padding, the
//! `enabled` flag, `Default`, the `PostProcessEffect` impl pointing at the
//! embedded shader, and the inspector section. What is left here is the two
//! things only this effect knows: its fields, and its shader.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// Sepia settings. The macro appends `enabled` and pads the uniform out to two
/// `vec4`s, so `sepia.wgsl`'s `SepiaSettings` must match field for field —
/// nothing checks it at run time, and a mismatch is not an error but a wrong
/// picture, every field from the mismatch onward reading its neighbour's value.
#[post_process(shader = "sepia.wgsl", name = "Sepia", icon = "drop-half")]
pub struct Sepia {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 1.0)]
    pub intensity: f32,
    #[field(skip, default = 1.2)]
    pub tone_r: f32,
    #[field(skip, default = 1.0)]
    pub tone_g: f32,
    #[field(skip, default = 0.8)]
    pub tone_b: f32,
}

#[derive(Default)]
pub struct SepiaPlugin;

impl Plugin for SepiaPlugin {
    fn build(&self, app: &mut App) {
        // The shader travels inside the compiled library rather than beside it,
        // so there is no asset path for an installed plugin to get wrong.
        bevy::asset::embedded_asset!(app, "sepia.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Sepia>::default());
        app.register_inspectable::<Sepia>();
    }
}

// `Runtime`, explicitly: `plugin!` defaults to `Editor`, which would show the
// effect in the editor viewport and ship none of it to a game.
renzora::plugin!(SepiaPlugin, Runtime);
