//! Wipe and fade screen transition post-process effect.
//!
//! `order = 2.0` puts it after everything, `pulse` included. A transition is the
//! last thing between the finished frame and the player: an effect running after
//! it would filter the wipe itself, so a fade to black would come out grey.
//!
//! `progress` defaults to `1.0`, meaning fully revealed. A transition that
//! defaulted to `0.0` would blank the screen the moment it was added.
//!
//! This is the one effect in the set that needs the framework's snapshot: the
//! outgoing frame is a copy of the previous fully-composited frame, bound at
//! `@binding(3)`, which the pipeline only provides when the effect asks for it.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `screen_transition.wgsl`'s `ScreenTransitionSettings` must match field for
/// field.
#[post_process(
    shader = "screen_transition.wgsl",
    name = "Screen Transition",
    icon = "arrows-left-right",
    order = 2.0,
    // The shader samples `@binding(3)`/`@binding(4)`, so the pipeline has to
    // carry them. Without this it is a wgpu validation crash at pipeline
    // creation, not a missing picture.
    snapshot = true,
    // Freeze the outgoing frame for as long as the wipe is running. At 1.0 the
    // transition has finished, so the snapshot goes back to tracking the live
    // frame and is ready for the next one.
    frozen_when = "self.progress < 1.0"
)]
pub struct ScreenTransition {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 1.0)]
    pub progress: f32,
    #[field(min = 0.0, max = 3.0, speed = 1.0, default = 0.0)]
    pub mode: f32,
    #[field(min = 0.0, max = 3.0, speed = 1.0, default = 0.0)]
    pub direction: f32,
    #[field(min = 0.0, max = 0.5, speed = 0.005, default = 0.03)]
    pub smoothness: f32,
    #[field(skip, default = 0.0)]
    pub color_r: f32,
    #[field(skip, default = 0.0)]
    pub color_g: f32,
    #[field(skip, default = 0.0)]
    pub color_b: f32,
}

#[derive(Default)]
pub struct ScreenTransitionPlugin;

impl Plugin for ScreenTransitionPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "screen_transition.wgsl");
        app.add_plugins(
            renzora::postprocess::PostProcessPlugin::<ScreenTransition>::default(),
        );
        app.register_inspectable::<ScreenTransition>();
    }
}

renzora::plugin!(ScreenTransitionPlugin, Runtime);
