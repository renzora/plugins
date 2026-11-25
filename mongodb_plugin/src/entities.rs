//! Entities to and from MongoDB documents.
//!
//! Two shapes:
//!
//! - **One entity per document** ([`spawn_docs`]): a document's well-known
//!   fields place it (`name`, `x`, `y`, `z`, `rx`, `ry`, `rz`, `scale`), give it
//!   a look (`prefab`, or `shape` and `color`), and a `components` object can
//!   carry any reflected component. The whole document stays on the entity in
//!   [`MongoDoc`], so a script reads the game's own fields (`hp`, `owner`)
//!   from it later.
//! - **One entity tree per document** ([`tree_to_json`], [`spawn_tree`]): an
//!   entity, its children and every component that can be written, stored as
//!   a nested document.
//!
//! Through reflection, so it works for any component the game registered,
//! including a plugin's. The same code as the MySQL plugin's, kept as a copy:
//! two marketplace plugins cannot share a crate without a third.

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::reflect::serde::{TypedReflectDeserializer, TypedReflectSerializer};
use bevy::reflect::TypeRegistry;
use renzora::serde::de::DeserializeSeed;
use renzora::serde_json::{self, Map, Value};

/// The document an entity was spawned from, as relaxed Extended JSON.
/// Reflected, so a script reads a field with the engine's field reader
/// (`MongoDoc.data`) and a scene save keeps it.
#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default)]
pub struct MongoDoc {
    /// The document: one JSON object.
    pub data: String,
}

/// Components that are not the entity's own data: recomputed every frame,
/// rebuilt from other components, bound to this session, or a link to another
/// entity that would not mean the same thing when spawned again.
const SKIP: &[&str] = &[
    "bevy_ecs::hierarchy::",
    "bevy_transform::components::global_transform::",
    "bevy_transform::components::transform::TransformTreeChanged",
    "bevy_camera::visibility::InheritedVisibility",
    "bevy_camera::visibility::ViewVisibility",
    "bevy_camera::visibility::VisibleEntities",
    "bevy_camera::visibility::VisibleMeshEntities",
    "bevy_camera::primitives::",
    "bevy_mesh::",
    "bevy_pbr::",
    "bevy_render::",
    "bevy_asset::",
    "bevy_ui::ui_node::ComputedNode",
    "bevy_ui::ui_transform::UiGlobalTransform",
    "bevy_ui::stack::ComputedStackIndex",
    "bevy_ui::ui_node::ComputedUiRenderTargetInfo",
    "bevy_ui::measurement::ContentSize",
    "bevy_text::",
    "renzora::core::components::SceneEntityId",
    "renzora_network::components::NetworkId",
    "renzora_physics::read_state::",
    // The physics engine's own state: colliders, bodies, contacts and the
    // mass it computed, all rebuilt from `PhysicsBodyData` and
    // `CollisionShapeData` on spawn. Two of them (`ColliderOf`,
    // `RigidBodyColliders`) also name other entities, and written back onto a
    // new entity they would point at whatever now has the old ids.
    "avian2d::",
    "avian3d::",
];

fn skipped(path: &str) -> bool {
    SKIP.iter().any(|p| path.starts_with(p))
}

fn num(row: &Map<String, Value>, key: &str) -> Option<f32> {
    match row.get(key)? {
        Value::Number(n) => n.as_f64().map(|f| f as f32),
        // Decimal columns arrive as text.
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn text<'a>(row: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    row.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// `#rrggbb`, `#rrggbbaa`, or `r,g,b[,a]` in 0..1.
fn color(s: &str) -> Option<Color> {
    if let Some(hex) = s.strip_prefix('#') {
        return Srgba::hex(hex).ok().map(Color::from);
    }
    let parts: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [r, g, b] => Some(Color::srgb(*r, *g, *b)),
        [r, g, b, a] => Some(Color::srgba(*r, *g, *b, *a)),
        _ => None,
    }
}

/// Spawn one entity per document, under `parent` if given. Returns them in
/// order, with any per-document problem (an unknown component) as a warning.
pub fn spawn_docs(world: &mut World, rows: &[Map<String, Value>], parent: Option<Entity>) -> (Vec<Entity>, Vec<String>) {
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let mut spawned = Vec::new();
    let mut warnings = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let name = text(row, "name").map(str::to_string).unwrap_or_else(|| format!("doc_{i}"));
        let mut t = Transform::from_xyz(
            num(row, "x").unwrap_or(0.0),
            num(row, "y").unwrap_or(0.0),
            num(row, "z").unwrap_or(0.0),
        );
        let (rx, ry, rz) = (num(row, "rx"), num(row, "ry"), num(row, "rz"));
        if rx.is_some() || ry.is_some() || rz.is_some() {
            t.rotation = Quat::from_euler(
                EulerRot::YXZ,
                ry.unwrap_or(0.0).to_radians(),
                rx.unwrap_or(0.0).to_radians(),
                rz.unwrap_or(0.0).to_radians(),
            );
        }
        if let Some(s) = num(row, "scale") {
            t.scale = Vec3::splat(s);
        }

        let mut e = world.spawn((
            Name::new(name),
            t,
            Visibility::default(),
            MongoDoc { data: Value::Object(row.clone()).to_string() },
        ));
        if let Some(prefab) = text(row, "prefab") {
            e.insert(renzora::core::SceneInstance { source: prefab.to_string(), ..Default::default() });
        } else if let Some(shape) = text(row, "shape") {
            e.insert(renzora::core::MeshPrimitive(shape.to_string()));
            if let Some(c) = text(row, "color").and_then(color) {
                e.insert(renzora::core::MeshColor(c));
            }
        }
        if let Some(p) = parent {
            e.insert(ChildOf(p));
        }
        let id = e.id();

        // `components`: { "type::path::Or::ShortName": value, ... }
        let components = match row.get("components") {
            Some(Value::String(s)) if !s.trim().is_empty() => match serde_json::from_str::<Value>(s) {
                Ok(v) => Some(v),
                Err(err) => {
                    warnings.push(format!("document {i}: `components` is not JSON: {err}"));
                    None
                }
            },
            Some(v @ Value::Object(_)) => Some(v.clone()),
            _ => None,
        };
        if let Some(Value::Object(map)) = components {
            for (path, value) in map {
                if let Err(err) = insert_component(world, &registry, id, &path, value) {
                    warnings.push(format!("document {i}: {err}"));
                }
            }
        }
        spawned.push(id);
    }
    (spawned, warnings)
}

/// Find a component's registration by full type path, or by short name when
/// that is unambiguous (`"Health"` for `my_game::Health`).
fn registration<'r>(registry: &'r TypeRegistry, path: &str) -> Result<&'r bevy::reflect::TypeRegistration, String> {
    if let Some(r) = registry.get_with_type_path(path) {
        return Ok(r);
    }
    let mut found = registry
        .iter()
        .filter(|r| r.data::<ReflectComponent>().is_some() && r.type_info().type_path_table().short_path() == path);
    match (found.next(), found.next()) {
        (Some(r), None) => Ok(r),
        (Some(_), Some(_)) => Err(format!("`{path}` names more than one component; use its full type path")),
        _ => Err(format!("`{path}` is not a registered component")),
    }
}

fn insert_component(world: &mut World, registry: &TypeRegistry, entity: Entity, path: &str, value: Value) -> Result<(), String> {
    let reg = registration(registry, path)?;
    let full = reg.type_info().type_path();
    if skipped(full) || reg.data::<bevy::ecs::reflect::ReflectMapEntities>().is_some() {
        // Silently: a row saved by an older version of this plugin carries
        // them, and they are rebuilt or meaningless here either way.
        return Ok(());
    }
    let rc = reg.data::<ReflectComponent>().ok_or_else(|| format!("`{path}` is not a component"))?;
    let reflected = TypedReflectDeserializer::new(reg, registry)
        .deserialize(value)
        .map_err(|e| format!("`{path}`: {e}"))?;
    let mut em = world.get_entity_mut(entity).map_err(|_| "the entity is gone".to_string())?;
    rc.insert(&mut em, reflected.as_ref(), registry);
    Ok(())
}

/// The entity and everything under it as JSON: `{ "format": 1, "entities":
/// [ { "parent": index or null, "components": { path: value } } ] }`, the root
/// first.
pub fn tree_to_json(world: &World, root: Entity) -> Result<String, String> {
    if world.get_entity(root).is_err() {
        return Err("that entity does not exist".into());
    }
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let mut order: Vec<(Entity, Option<usize>)> = vec![(root, None)];
    let mut i = 0;
    while i < order.len() {
        let (e, _) = order[i];
        // A model's insides are rebuilt from its file on spawn; written out
        // too, they would come back twice.
        let is_model = world.get::<renzora::core::MeshInstanceData>(e).is_some();
        if !is_model {
            if let Some(children) = world.get::<Children>(e) {
                for c in children.iter() {
                    if world.get::<renzora::core::HideInHierarchy>(c).is_none() {
                        order.push((c, Some(i)));
                    }
                }
            }
        }
        i += 1;
    }

    let mut entities = Vec::new();
    for (e, parent) in &order {
        let mut components = Map::new();
        let er = world.entity(*e);
        for (_, info) in world.inspect_entity(*e).into_iter().flatten() {
            let Some(type_id) = info.type_id() else { continue };
            let Some(reg) = registry.get(type_id) else { continue };
            let path = reg.type_info().type_path();
            // Anything that declares it holds entity ids (a joint, a
            // relationship) would point at the wrong entities when spawned
            // again, since the saved ids are not the new ones.
            if skipped(path) || reg.data::<bevy::ecs::reflect::ReflectMapEntities>().is_some() {
                continue;
            }
            let Some(rc) = reg.data::<ReflectComponent>() else { continue };
            let Some(value) = rc.reflect(er) else { continue };
            let ser = TypedReflectSerializer::new(value.as_partial_reflect(), &registry);
            // A component that cannot be written (it holds a handle or an
            // entity) is left out rather than failing the whole tree.
            if let Ok(v) = serde_json::to_value(&ser) {
                components.insert(path.to_string(), v);
            }
        }
        entities.push(serde_json::json!({ "parent": parent, "components": components }));
    }
    Ok(serde_json::json!({ "format": 1, "entities": entities }).to_string())
}

/// Spawn a tree written by [`tree_to_json`], under `parent` if given. Returns
/// the root and any per-component problems.
pub fn spawn_tree(world: &mut World, text: &str, parent: Option<Entity>) -> Result<(Entity, Vec<String>), String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("the saved entity is not JSON: {e}"))?;
    let list = doc
        .get("entities")
        .and_then(Value::as_array)
        .filter(|l| !l.is_empty())
        .ok_or("the saved entity holds no entities")?;
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let mut ids: Vec<Entity> = Vec::with_capacity(list.len());
    let mut warnings = Vec::new();
    for item in list {
        let id = world.spawn_empty().id();
        ids.push(id);
        if let Some(Value::Object(components)) = item.get("components") {
            for (path, value) in components {
                if let Err(e) = insert_component(world, &registry, id, path, value.clone()) {
                    warnings.push(e);
                }
            }
        }
    }
    for (i, item) in list.iter().enumerate() {
        let p = match item.get("parent").and_then(Value::as_u64) {
            Some(idx) => ids.get(idx as usize).copied(),
            None => parent,
        };
        if let Some(p) = p {
            world.entity_mut(ids[i]).insert(ChildOf(p));
        }
    }
    Ok((ids[0], warnings))
}
