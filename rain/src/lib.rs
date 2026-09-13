//! Rain-on-lens post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled` and pads the uniform out to two `vec4`s, so
/// `rain.wgsl`'s `RainSettings` must match field for field.
#[post_process(shader = "rain.wgsl", name = "Rain", icon = "cloud-rain")]
pub struct Rain {
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.3)]
    pub intensity: f32,
    #[field(min = 0.1, max = 5.0, speed = 0.1, default = 1.0)]
    pub speed: f32,
    /// How big each bead is. The shader turns this into a grid frequency, which
    /// is its inverse — bigger beads means fewer of them across the screen.
    #[field(min = 1.0, max = 20.0, speed = 0.1, default = 8.0)]
    pub drop_size: f32,
    /// How much the fall rate varies between columns. 0 makes every bead run
    /// at exactly `speed`; 1 spreads them from a crawl to nearly double it.
    ///
    /// The variation is per column rather than per bead, and has to be: which
    /// bead a column is on is worked out from how far it has fallen, so the
    /// rate has to be known before there is a bead to ask about.
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub speed_variation: f32,
    /// Length of the runnel each bead drags behind it, as a multiple of its own
    /// size. 0 leaves bare beads.
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.5)]
    pub trail: f32,
    /// How much the glass fogs between the water. 0 leaves it clear; the water
    /// wipes the fog away wherever it runs, which is most of what makes this
    /// read as a wet windscreen rather than marks on the picture.
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.6)]
    pub fog: f32,
    /// Seconds, advanced by [`sync_time`].
    #[field(skip, default = 0.0)]
    pub time: f32,
}

/// Drives the shader's clock.
///
/// Nothing was writing this before, which left the drops hanging still on the
/// lens: the framework uploads the component's bytes verbatim and interprets no
/// field.
fn sync_time(mut q: Query<&mut Rain>, time: Res<Time>) {
    for mut s in &mut q {
        s.time += time.delta_secs();
        if s.time > 1024.0 {
            s.time -= 1024.0;
        }
    }
}

#[derive(Default)]
pub struct RainPlugin;

impl Plugin for RainPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "rain.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Rain>::default());
        app.add_systems(Update, sync_time);
        app.register_inspectable::<Rain>();
    }
}

renzora::plugin!(RainPlugin, Runtime);
