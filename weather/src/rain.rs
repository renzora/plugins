//! Rain around the camera.
//!
//! The drops are the engine's own particle system, driving an effect embedded
//! in this plugin. `ParticleSystem` lives in `renzora_particles`, which a
//! native plugin cannot link, so the emitter is built and switched through
//! reflection, the same way the cloud deck is (see [`crate::clouds`]).

use bevy::camera::visibility::RenderLayers;
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::enums::{DynamicEnum, DynamicVariant};
use bevy::reflect::std_traits::ReflectDefault;
use bevy::reflect::GetPath;

use crate::data::{Weather, WeatherState};

/// The rain effect, embedded in this plugin.
pub const RAIN_EFFECT: &str = "embedded://weather/rain.particle";

/// Rain below this falls too thinly to be worth drawing.
const RAIN_VISIBLE: f32 = 0.15;

/// Height above the camera the rain is emitted from, in metres: high enough
/// that drops are already falling at speed when they come into view.
const RAIN_HEIGHT: f32 = 14.0;

/// The rain emitter. Never saved: it has no `Name`, which the scene saver
/// requires, and the hierarchy skips it.
#[derive(Component)]
pub struct RainEmitter;

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

/// Keep the rain over the camera, playing while it rains.
pub fn follow_rain(world: &mut World) {
    let rain = world.resource::<WeatherState>().rain;
    let mut weathers = world.query::<&Weather>();
    let any_enabled = weathers.iter(world).any(|w| w.enabled);
    let wanted = weathers.iter(world).any(|w| w.enabled && w.rain_particles) && rain > RAIN_VISIBLE;
    let camera = camera_position(world);

    let mut emitters = world.query_filtered::<Entity, With<RainEmitter>>();
    let emitter = emitters.iter(world).next();
    let Some(emitter) = emitter else {
        if wanted {
            let at = camera.unwrap_or_default() + Vec3::Y * RAIN_HEIGHT;
            let entity = world.spawn((RainEmitter, renzora::HideInHierarchy, Transform::from_translation(at))).id();
            if !insert_particle_system(world, entity, RAIN_EFFECT) {
                // No particle system in this build. Keep the empty emitter so
                // this is not retried, and say so once.
                renzora::console_log::console_warn(
                    "Weather",
                    "Rain needs the engine's particle system, which this build does not have",
                );
            }
        }
        return;
    };
    if !any_enabled {
        world.despawn(emitter);
        return;
    }
    set_play_state(world, emitter, if wanted { "Playing" } else { "Stopped" });

    let heading = world.get_resource::<renzora::WindState>().map(|w| w.direction);
    if let Some(mut transform) = world.get_mut::<Transform>(emitter) {
        if let Some(cam) = camera {
            transform.translation = cam + Vec3::Y * RAIN_HEIGHT;
        }
        // Lean the rain with the wind. The effect slants its drops along local
        // +X, so turning the emitter turns the slant.
        if let Some(h) = heading.filter(|h| h.length_squared() > 1e-6) {
            transform.rotation = Quat::from_rotation_y((-h.y).atan2(h.x));
        }
    }
}
