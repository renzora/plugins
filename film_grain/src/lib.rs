//! Animated film grain post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `film_grain.wgsl`'s `FilmGrainSettings` must match field for field.
#[post_process(shader = "film_grain.wgsl", name = "Film Grain", icon = "film-strip")]
pub struct FilmGrain {
    #[field(min = 0.0, max = 2.0, speed = 0.01, default = 0.3)]
    pub intensity: f32,
    /// Width of one grain cell, in **pixels**. Below 1.0 there is nothing left to
    /// resolve, one cell per pixel being as fine as the grain gets, so the range
    /// starts there.
    #[field(min = 1.0, max = 10.0, speed = 0.1, default = 1.5)]
    pub grain_size: f32,
    /// Seconds, advanced by [`sync_time`]. Skipped in the inspector but *not*
    /// engine-driven: nothing in the host writes this, and leaving it at its
    /// default is what froze the grain into a fixed dirt layer.
    #[field(skip, default = 0.0)]
    pub time: f32,
}

/// Drives the shader's clock.
///
/// A post-process effect gets no time for free: the framework uploads the
/// component's bytes verbatim without interpreting a single field, so an
/// animated effect has to tick its own value.
fn sync_time(mut q: Query<&mut FilmGrain>, time: Res<Time>) {
    for mut g in &mut q {
        // Accumulated and wrapped rather than assigned from `elapsed_secs`. The
        // shader quantises this to a 24 Hz frame index, and an f32 second count
        // stops resolving those steps after a few hours of uptime; wrapping keeps
        // the seed small. A grain pattern that repeats every ~17 minutes is not
        // something anyone can see.
        g.time += time.delta_secs();
        if g.time > 1024.0 {
            g.time -= 1024.0;
        }
    }
}

#[derive(Default)]
pub struct FilmGrainPlugin;

impl Plugin for FilmGrainPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "film_grain.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<FilmGrain>::default());
        app.add_systems(Update, sync_time);
        app.register_inspectable::<FilmGrain>();
    }
}

renzora::plugin!(FilmGrainPlugin, Runtime);
