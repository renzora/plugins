//! The cloud deck, reached through reflection.
//!
//! `CloudsData` lives in the engine's `renzora_clouds` crate, which a native
//! plugin cannot link: it reaches `bevy`, `renzora` and `renzora_ember` and
//! nothing else. The component is registered for reflection (that is how a
//! scene saves it), so the plugin finds it in the type registry by name and
//! reads and writes the fields it drives by path. A build without clouds, or a
//! future rename, leaves the registry without it and the weather simply drives
//! the sun, haze and wind alone.

use bevy::ecs::component::ComponentId;
use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::GetPath;

/// The short type name the deck is registered under.
const CLOUDS_TYPE: &str = "CloudsData";

/// The fields weather drives, as plain values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clouds {
    pub coverage: f32,
    pub density: f32,
    pub bottom_height: f32,
    pub top_height: f32,
    pub absorption: f32,
    pub color: Vec3,
    pub shadow_color: Vec3,
    pub brightness: f32,
    pub ambient_brightness: f32,
    pub edge_softness: f32,
    pub detail_strength: f32,
    pub morph_speed: f32,
    pub speed: f32,
}

const SCALARS: [&str; 11] = [
    "coverage",
    "density",
    "bottom_height",
    "top_height",
    "absorption",
    "brightness",
    "ambient_brightness",
    "edge_softness",
    "detail_strength",
    "morph_speed",
    "speed",
];

impl Clouds {
    fn scalars(&self) -> [f32; 11] {
        [
            self.coverage,
            self.density,
            self.bottom_height,
            self.top_height,
            self.absorption,
            self.brightness,
            self.ambient_brightness,
            self.edge_softness,
            self.detail_strength,
            self.morph_speed,
            self.speed,
        ]
    }
}

/// An engine component found by its short type name, for reading and writing
/// by field path. `None` when this build does not have it.
pub fn find_component(world: &World, short_name: &str) -> Option<(ComponentId, ReflectComponent)> {
    let registry = world.get_resource::<AppTypeRegistry>()?.read();
    let registration = registry.get_with_short_type_path(short_name)?;
    let reflect = registration.data::<ReflectComponent>()?.clone();
    let id = world.components().get_id(registration.type_id())?;
    Some((id, reflect))
}

fn component_id(world: &World) -> Option<(ComponentId, ReflectComponent)> {
    find_component(world, CLOUDS_TYPE)
}

/// The first entity carrying the component `short_name`, with the `f32` at
/// `field` on it.
pub fn read_scalar(world: &mut World, short_name: &str, field: &str) -> Option<(Entity, f32)> {
    let (id, reflect) = find_component(world, short_name)?;
    let mut query = QueryBuilder::<Entity>::new(world).with_id(id).build();
    let entity = query.iter(world).next()?;
    let value = reflect.reflect(world.get_entity(entity).ok()?)?;
    Some((entity, read_f32(value, field)?))
}

/// Write the `f32` at `field` of the component `short_name` on `entity`,
/// leaving it untouched (and unchanged in Bevy's eyes) if it already holds `v`.
pub fn write_scalar(world: &mut World, entity: Entity, short_name: &str, field: &str, v: f32) {
    let Some((_, reflect)) = find_component(world, short_name) else { return };
    let current = world
        .get_entity(entity)
        .ok()
        .and_then(|e| reflect.reflect(e))
        .and_then(|value| read_f32(value, field));
    if current == Some(v) {
        return;
    }
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else { return };
    if let Some(mut value) = reflect.reflect_mut(&mut entity_mut) {
        write_f32(&mut *value, field, v);
    }
}

fn read_f32(value: &dyn Reflect, path: &str) -> Option<f32> {
    value.reflect_path(path).ok()?.try_downcast_ref::<f32>().copied()
}

fn read_color(value: &dyn Reflect, path: &str) -> Option<Vec3> {
    Some(Vec3::new(
        read_f32(value, &format!("{path}.0"))?,
        read_f32(value, &format!("{path}.1"))?,
        read_f32(value, &format!("{path}.2"))?,
    ))
}

fn write_f32(value: &mut dyn Reflect, path: &str, v: f32) {
    if let Ok(field) = value.reflect_path_mut(path) {
        if let Some(f) = field.try_downcast_mut::<f32>() {
            *f = v;
        }
    }
}

/// The first enabled deck in the scene, with its entity.
pub fn read(world: &mut World) -> Option<(Entity, Clouds)> {
    let (id, reflect) = component_id(world)?;
    let mut query = QueryBuilder::<Entity>::new(world).with_id(id).build();
    let entities: Vec<Entity> = query.iter(world).collect();
    for entity in entities {
        let Ok(entity_ref) = world.get_entity(entity) else { continue };
        let Some(value) = reflect.reflect(entity_ref) else { continue };
        let enabled = value
            .reflect_path("enabled")
            .ok()
            .and_then(|f| f.try_downcast_ref::<bool>().copied())
            .unwrap_or(true);
        if !enabled {
            continue;
        }
        let f = |p: &str| read_f32(value, p);
        let clouds = Clouds {
            coverage: f("coverage")?,
            density: f("density")?,
            bottom_height: f("bottom_height")?,
            top_height: f("top_height")?,
            absorption: f("absorption")?,
            color: read_color(value, "color")?,
            shadow_color: read_color(value, "shadow_color")?,
            brightness: f("brightness")?,
            ambient_brightness: f("ambient_brightness")?,
            edge_softness: f("edge_softness")?,
            detail_strength: f("detail_strength")?,
            morph_speed: f("morph_speed")?,
            speed: f("speed")?,
        };
        return Some((entity, clouds));
    }
    None
}

/// Write `clouds` into the deck on `entity`.
pub fn write(world: &mut World, entity: Entity, clouds: &Clouds) {
    let Some((_, reflect)) = component_id(world) else { return };
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else { return };
    let Some(mut value) = reflect.reflect_mut(&mut entity_mut) else { return };
    let value: &mut dyn Reflect = &mut *value;
    for (path, v) in SCALARS.iter().zip(clouds.scalars()) {
        write_f32(value, path, v);
    }
    for (path, c) in [("color", clouds.color), ("shadow_color", clouds.shadow_color)] {
        write_f32(value, &format!("{path}.0"), c.x);
        write_f32(value, &format!("{path}.1"), c.y);
        write_f32(value, &format!("{path}.2"), c.z);
    }
}
