//! Underwater caustics and tint post-process effect.

use bevy::prelude::*;
use renzora::{post_process, AppEditorExt};

/// The macro appends `enabled`, so `underwater.wgsl`'s `UnderwaterSettings` must
/// match field for field.
#[post_process(shader = "underwater.wgsl", name = "Underwater", icon = "wave-triangle")]
pub struct Underwater {
    #[field(min = 0.0, max = 2.0, speed = 0.01, default = 0.02)]
    pub distortion: f32,
    #[field(skip, default = 0.0)]
    pub tint_r: f32,
    #[field(skip, default = 0.3)]
    pub tint_g: f32,
    #[field(skip, default = 0.5)]
    pub tint_b: f32,
    #[field(min = 0.0, max = 1.0, speed = 0.01, default = 0.3)]
    pub tint_strength: f32,
    #[field(min = 0.0, max = 10.0, speed = 0.01, default = 1.0)]
    pub wave_speed: f32,
    #[field(min = 0.1, max = 50.0, speed = 0.1, default = 10.0)]
    pub wave_scale: f32,
    /// Seconds, advanced by [`sync_time`].
    #[field(skip, default = 0.0)]
    pub time: f32,
}

/// Drives the shader's clock.
///
/// Nothing was writing this before, which left the caustics frozen: the
/// framework uploads the component's bytes verbatim and interprets no field.
fn sync_time(mut q: Query<&mut Underwater>, time: Res<Time>) {
    for mut s in &mut q {
        s.time += time.delta_secs();
        if s.time > 1024.0 {
            s.time -= 1024.0;
        }
    }
}

#[derive(Default)]
pub struct UnderwaterPlugin;

impl Plugin for UnderwaterPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "underwater.wgsl");
        app.add_plugins(renzora::postprocess::PostProcessPlugin::<Underwater>::default());
        app.add_systems(Update, sync_time);
        app.register_inspectable::<Underwater>();
    }
}

renzora::plugin!(UnderwaterPlugin, Runtime);
