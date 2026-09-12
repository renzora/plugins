//! Pulsing vignette post-process effect.
//!
//! The shader used to be an inline `const WGSL: &str`. It is a file now because
//! `embedded_asset!` takes a path: the WGSL still travels inside the compiled
//! library, so nothing about distribution changes, but the compiler no longer
//! has to be handed it as a string literal.
//!
//! `order = 1.0` is load-bearing. Everything else in the set sorts at `0.0`, and
//! this darkens toward the edges: run it first and the other filters work on an
//! already-vignetted picture, spreading the darkening into whatever they do. It
//! has to land last.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `pulse.wgsl`'s `PulseSettings` must match field for field.
#[post_process(shader = "pulse.wgsl", name = "Pulse", icon = "pulse", order = 1.0)]
pub struct Pulse {
    #[field(min = 0.0, max = 2.0, speed = 0.01, default = 0.6)]
    pub strength: f32,
    #[field(min = 0.0, max = 10.0, speed = 0.01, default = 2.0)]
    pub speed: f32,
    /// Advanced by [`tick`] each frame.
    #[field(skip, default = 0.0)]
    pub time: f32,
}

/// Drives the shader's clock, scaled by the authored `speed`.
fn tick(mut q: Query<&mut Pulse>, time: Res<Time>) {
    for mut p in &mut q {
        let step = p.speed * time.delta_secs();
        p.time += step;
        // Wrapped on a multiple of TAU so the sine the shader takes of it is
        // continuous across the wrap; a plain clamp would jump the phase.
        if p.time > core::f32::consts::TAU * 1024.0 {
            p.time -= core::f32::consts::TAU * 1024.0;
        }
    }
}

#[derive(Default)]
pub struct PulsePlugin;

impl Plugin for PulsePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "pulse.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Pulse>::default());
        app.add_systems(Update, tick);
        app.register_inspectable::<Pulse>();
    }
}

renzora::plugin!(PulsePlugin, Runtime);
