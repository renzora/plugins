//! Rain and snow around the camera.
//!
//! Both are the engine's own particle system, driving effects embedded in
//! this plugin. `ParticleSystem` lives in `renzora_particles`, which a native
//! plugin cannot link, so the emitters are built and switched through
//! reflection, the same way the cloud deck is (see [`crate::clouds`]).
//!
//! One set of emitters per kind rather than one effect swapped between them,
//! so a change from rain to snow crossfades: the last raindrops are still
//! falling when the first flakes arrive, instead of every drop vanishing at
//! once.
//!
//! **Harder rain is more layers.** A particle effect's spawn rate is fixed in
//! its file, so how hard it falls is how many copies of the effect play, up to
//! [`LAYERS`], each shifted sideways so their identical random streams never
//! line up drop for drop.

use bevy::camera::visibility::RenderLayers;
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::enums::{DynamicEnum, DynamicVariant};
use bevy::reflect::std_traits::ReflectDefault;
use bevy::reflect::GetPath;

use crate::data::{Weather, WeatherState};

/// What falls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Precipitation {
    Rain,
    Snow,
}

/// One emitter: which kind, and which of its layers. Never saved: it has no
/// `Name`, which the scene saver requires, and the hierarchy skips it.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct PrecipitationEmitter {
    pub kind: Precipitation,
    pub layer: usize,
}

/// Most copies of an effect that play at once.
pub const LAYERS: usize = 3;

/// How much has to be falling for each layer to play. One layer is a light
/// shower, two the preset as tuned, three a doubled amount.
const LAYER_THRESHOLDS: [f32; LAYERS] = [0.15, 0.6, 1.3];

/// Where each layer sits relative to the camera, so their drops interleave.
const LAYER_OFFSETS: [Vec2; LAYERS] = [Vec2::ZERO, Vec2::new(6.3, 2.9), Vec2::new(-4.1, 7.7)];

impl Precipitation {
    const ALL: [Precipitation; 2] = [Precipitation::Rain, Precipitation::Snow];

    fn effect(self) -> &'static str {
        match self {
            Self::Rain => "embedded://weather/rain.particle",
            Self::Snow => "embedded://weather/snow.particle",
        }
    }

    /// Height above the camera it is emitted from, in metres: high enough
    /// that it is already falling at speed (or drifting, for snow) when it
    /// comes into view.
    fn height(self) -> f32 {
        match self {
            Self::Rain => 14.0,
            Self::Snow => 12.0,
        }
    }

    fn amount(self, state: &WeatherState) -> f32 {
        match self {
            Self::Rain => state.rain,
            Self::Snow => state.snow,
        }
    }
}

fn lights_the_scene(layers: Option<&RenderLayers>) -> bool {
    layers.is_none_or(|l| l.intersects(&RenderLayers::default()))
}

/// The camera the player is looking through: the first active one on the
/// scene's layer, which skips the editor's offscreen thumbnail rigs.
fn camera_position(world: &mut World) -> Option<Vec3> {
    let mut q = world.query_filtered::<(&GlobalTransform, &Camera, Option<&RenderLayers>), With<Camera3d>>();
    q.iter(world)
        .find(|(_, c, l)| c.is_active && lights_the_scene(*l))
        .map(|(t, _, _)| t.translation())
}

/// Insert the engine's `ParticleSystem` playing `path`, by reflection.
fn insert_particle_system(world: &mut World, entity: Entity, path: &str) -> bool {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let Some(registration) = registry.get_with_short_type_path("ParticleSystem") else {
        return false;
    };
    let (Some(reflect_component), Some(reflect_default)) =
        (registration.data::<ReflectComponent>(), registration.data::<ReflectDefault>())
    else {
        return false;
    };
    let mut value = reflect_default.default();
    match value.reflect_path_mut("path").ok().and_then(|f| f.try_downcast_mut::<String>()) {
        Some(p) => *p = path.to_string(),
        None => return false,
    }
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return false;
    };
    reflect_component.insert(&mut entity_mut, value.as_partial_reflect(), &registry);
    true
}

/// Set the emitter's `ParticlePlayback.state` to the variant named `state`,
/// if it is not already.
fn set_play_state(world: &mut World, entity: Entity, state: &str) {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let Some(reflect_component) = registry
        .get_with_short_type_path("ParticlePlayback")
        .and_then(|r| r.data::<ReflectComponent>())
    else {
        return;
    };
    // Read first: writing every frame would mark the playback changed every
    // frame for nothing.
    let current = world
        .get_entity(entity)
        .ok()
        .and_then(|e| reflect_component.reflect(e))
        .and_then(|p| match p.reflect_path("state").ok()?.reflect_ref() {
            bevy::reflect::ReflectRef::Enum(e) => Some(e.variant_name().to_string()),
            _ => None,
        });
    if current.as_deref() == Some(state) {
        return;
    }
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else { return };
    let Some(mut playback) = reflect_component.reflect_mut(&mut entity_mut) else { return };
    if let Ok(field) = playback.reflect_path_mut("state") {
        let _ = field.try_apply(&DynamicEnum::new(state, DynamicVariant::Unit));
    }
}

/// Keep each kind's emitters over the camera, as many playing as it is
/// falling hard.
pub fn follow_precipitation(world: &mut World) {
    let state = *world.resource::<WeatherState>();
    let mut weathers = world.query::<&Weather>();
    let active = weathers.iter(world).find(|w| w.enabled).cloned();
    let any_enabled = active.is_some();
    let shown = |kind: Precipitation| {
        active.as_ref().is_some_and(|w| match kind {
            Precipitation::Rain => w.rain_particles,
            Precipitation::Snow => w.snow_particles,
        })
    };
    let camera = camera_position(world);
    let heading = world.get_resource::<renzora::WindState>().map(|w| w.direction);

    for (kind, layer) in Precipitation::ALL.into_iter().flat_map(|k| (0..LAYERS).map(move |l| (k, l))) {
        let wanted = shown(kind) && kind.amount(&state) > LAYER_THRESHOLDS[layer];
        let offset = LAYER_OFFSETS[layer];
        let place = |cam: Vec3| cam + Vec3::new(offset.x, kind.height(), offset.y);
        let mut emitters = world.query::<(Entity, &PrecipitationEmitter)>();
        let emitter = emitters
            .iter(world)
            .find(|(_, e)| e.kind == kind && e.layer == layer)
            .map(|(e, _)| e);

        let Some(emitter) = emitter else {
            if wanted {
                let at = place(camera.unwrap_or_default());
                let entity = world
                    .spawn((
                        PrecipitationEmitter { kind, layer },
                        renzora::HideInHierarchy,
                        Transform::from_translation(at),
                    ))
                    .id();
                if !insert_particle_system(world, entity, kind.effect()) {
                    // No particle system in this build. Keep the empty emitter
                    // so this is not retried every frame, and say so once.
                    renzora::console_log::console_warn(
                        "Weather",
                        "Rain and snow need the engine's particle system, which this build does not have",
                    );
                }
            }
            continue;
        };
        if !any_enabled {
            world.despawn(emitter);
            continue;
        }
        set_play_state(world, emitter, if wanted { "Playing" } else { "Stopped" });

        if let Some(mut transform) = world.get_mut::<Transform>(emitter) {
            if let Some(cam) = camera {
                transform.translation = place(cam);
            }
            // Lean the rain with the wind. The effect slants its drops along
            // local +X, so turning the emitter turns the slant; snow falls
            // straight and drifts on its own noise, so turning it is harmless.
            if let Some(h) = heading.filter(|h| h.length_squared() > 1e-6) {
                transform.rotation = Quat::from_rotation_y((-h.y).atan2(h.x));
            }
        }
    }
}
