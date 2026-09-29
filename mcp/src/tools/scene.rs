//! Tools that make, move and remove entities: shapes, lights, colliders,
//! transforms, names, parents and copies.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
// The engine's own avian, through the contract crate's re-export. Naming it any
// other way would give this plugin a second `Collider` type with a different
// `TypeId`, which the physics step would never see.
use renzora::avian3d::prelude::{Collider, ColliderOf, RigidBody, RigidBodyColliders};
use renzora::serde_json::Value;
use renzora::{unique_entity_name, MeshColor, MeshPrimitive, ShapeRegistry};

use super::{arg_entity, arg_vec3, entity_id};
use crate::{edits, ToolReply};

/// A name that no other entity in the scene has.
///
/// Applied here rather than left to the engine, which renames a duplicate a
/// frame later: the reply said `pillar` while the hierarchy said `pillar_1`, so
/// an agent that found things by name was looking for one that did not exist.
fn unique_name(world: &mut World, entity: Entity, wanted: &str) -> String {
    let unique = unique_entity_name(world, wanted, entity);
    world.entity_mut(entity).insert(Name::new(unique.clone()));
    unique
}

fn name_of(world: &World, entity: Entity) -> String {
    world
        .get::<Name>(entity)
        .map(|n| n.as_str().to_string())
        .unwrap_or_default()
}

pub(super) fn spawn_entity(world: &mut World, args: &Value) -> ToolReply {
    let entity = match spawn_from_args(world, args) {
        Ok(entity) => entity,
        Err(why) => return ToolReply::Failed(why),
    };

    // Recorded so Ctrl+Z removes it. The redo closure keeps the arguments rather
    // than the entity, because a redone spawn is a NEW entity with a new id, and
    // the command updates itself with it.
    let name = name_of(world, entity);
    let again = args.clone();
    edits::record_spawn(world, entity, format!("spawn {name}"), move |world| {
        spawn_from_args(world, &again).ok()
    });

    ToolReply::Text(format!("spawned {name} [{}]", entity_id(entity)))
}

fn spawn_from_args(world: &mut World, args: &Value) -> Result<Entity, String> {
    let Some(name) = args.get("name").and_then(Value::as_str) else {
        return Err("spawn_entity needs a name".into());
    };
    // Checked before spawning, so a bad parent leaves nothing behind.
    let parent = arg_entity(args, "parent");
    if let Some(parent) = parent {
        if world.get_entity(parent).is_err() {
            return Err(format!("the parent {} does not exist", entity_id(parent)));
        }
    }

    let mut transform = Transform::from_translation(arg_vec3(args, "position").unwrap_or(Vec3::ZERO));
    if let Some(euler) = arg_vec3(args, "rotation") {
        transform.rotation = Quat::from_euler(
            EulerRot::XYZ,
            euler.x.to_radians(),
            euler.y.to_radians(),
            euler.z.to_radians(),
        );
    }
    if let Some(scale) = arg_vec3(args, "scale") {
        transform.scale = scale;
    }

    let shape = args.get("shape").and_then(Value::as_str).unwrap_or("none");

    let entity = if shape == "none" {
        // A bare transform, which is what an empty group or a marker wants.
        world.spawn(transform).id()
    } else {
        let Some(mesh) = mesh_handle(world, shape) else {
            return Err(format!(
                "no shape called {shape}; call list_shapes for what this project has"
            ));
        };
        let color = arg_vec3(args, "color").unwrap_or(Vec3::ONE);
        let linear = Color::linear_rgb(color.x, color.y, color.z);
        let material = material_handle(world, color);
        world
            .spawn((
                transform,
                Mesh3d(mesh),
                MeshMaterial3d(material),
                // The pair that survives a save. `Mesh3d` holds a handle to an
                // asset this plugin generated, which has no path on disk, so a
                // saved scene has nothing to point at and the entity comes back
                // as a transform with no geometry. That is not hypothetical: the
                // first arena built with this tool reloaded as 421 invisible
                // entities. The shape id and the colour are what the engine
                // rehydrates the mesh and material FROM.
                MeshPrimitive(shape.to_string()),
                MeshColor(linear),
            ))
            .id()
    };
    unique_name(world, entity, name);

    // Solid by default. A tool that hands back scenery you fall straight through
    // is a tool whose output has to be fixed by hand every time, and the caller
    // who wanted decoration can say so; the caller who wanted a floor should not
    // have to know they needed to ask.
    let solid = args
        .get("collider")
        .and_then(Value::as_bool)
        .unwrap_or(shape != "none");
    if solid {
        if let Some(collider) = collider_for(world, shape) {
            world
                .entity_mut(entity)
                .insert((collider, RigidBody::Static));
        }
    }

    // Inserting the relationship rather than calling `add_child`: `ChildOf` is
    // the source of truth in Bevy's relationship model and `Children` is kept in
    // step for us.
    if let Some(parent) = parent {
        world.entity_mut(entity).insert(ChildOf(parent));
    }

    Ok(entity)
}

/// Every shape the project has, as the shape library shows them.
///
/// Read from the registry rather than listed in the schema, because the list is
/// not this plugin's to know: `renzora_engine` registers about thirty, and a
/// plugin can add its own. A hardcoded enum was wrong the moment it was written
/// (it had five, and missed `stairs`, `ramp`, `arch` and `wedge`, which are
/// exactly what somebody building a level wants).
pub(super) fn list_shapes(world: &mut World) -> ToolReply {
    let Some(registry) = world.get_resource::<ShapeRegistry>() else {
        return ToolReply::Failed("this build has no shape registry".into());
    };
    let mut by_category: Vec<(String, String)> = registry
        .iter()
        .map(|entry| (entry.category.to_string(), entry.id.to_string()))
        .collect();
    if by_category.is_empty() {
        return ToolReply::Failed("no shapes are registered".into());
    }
    by_category.sort();

    let mut out = String::new();
    let mut current = String::new();
    for (category, id) in by_category {
        if category != current {
            out.push_str(&format!("\n{category}:\n  "));
            current = category;
        }
        out.push_str(&format!("{id} "));
    }
    ToolReply::Text(out.trim_start().to_string())
}

/// Meshes and materials already built, so a scene of five hundred crates is one
/// cube mesh and one brown material rather than five hundred of each.
///
/// Not a micro-optimisation. Bevy batches draws by handle, so a unique handle
/// per entity defeats batching outright: the first arena built with this plugin
/// was 415 entities carrying 415 meshes and 415 materials, and cost 3 ms a frame
/// that it did not need to.
#[derive(Resource, Default)]
pub struct AssetCache {
    meshes: HashMap<String, Handle<Mesh>>,
    /// Keyed by the colour quantised to 8 bits a channel. Two colours a caller
    /// wrote as 0.2 and 0.2000001 are the same material to anyone looking at it,
    /// and keying on raw `f32` would make them two.
    materials: HashMap<[u8; 3], Handle<StandardMaterial>>,
}

/// The mesh for a shape id, built by the engine's own factory.
///
/// Through [`ShapeRegistry`] rather than building a `Cuboid` here, so a cube
/// spawned by this plugin is the same mesh as a cube from the Shape Library:
/// same winding, same UVs, same size convention, and the same rehydration on
/// load. Two definitions of "cube" in one project is a bug waiting for whichever
/// of them is wrong.
fn mesh_handle(world: &mut World, shape: &str) -> Option<Handle<Mesh>> {
    if let Some(handle) = world.resource::<AssetCache>().meshes.get(shape) {
        return Some(handle.clone());
    }
    // The registry owns the factory and `Assets<Mesh>` is a separate resource,
    // so the two are taken one at a time rather than held together.
    let registry = world.get_resource::<ShapeRegistry>()?;
    let create = registry.get(shape)?.create_mesh;
    let handle = world.resource_scope(|_, mut meshes: Mut<Assets<Mesh>>| create(&mut meshes));
    world
        .resource_mut::<AssetCache>()
        .meshes
        .insert(shape.to_string(), handle.clone());
    Some(handle)
}

pub(crate) fn material_handle(world: &mut World, color: Vec3) -> Handle<StandardMaterial> {
    let key = [
        (color.x.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.y.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.z.clamp(0.0, 1.0) * 255.0).round() as u8,
    ];
    if let Some(handle) = world.resource::<AssetCache>().materials.get(&key) {
        return handle.clone();
    }
    let handle = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::linear_rgb(color.x, color.y, color.z),
            ..default()
        });
    world
        .resource_mut::<AssetCache>()
        .materials
        .insert(key, handle.clone());
    handle
}

/// A collider at unit size, to be scaled by the entity's own transform.
///
/// Unit rather than baked to the entity's scale because avian's
/// `transform_to_collider_scale` is on by default: it rescales the shape from
/// the transform every time that transform changes. Baking the size in here
/// would apply the scale twice, and a crate scaled to 0.7 would collide as 0.49.
///
/// The five analytic primitives are named because an exact shape is cheaper and
/// better behaved than a mesh of the same thing. Everything else in the registry
/// (`stairs`, `arch`, `ramp`, `quarter_pipe`, …) has no analytic equivalent, so
/// it gets a triangle mesh built from the mesh being drawn. That is sound here
/// only because all of this is static scenery: a trimesh is hollow, and a
/// *moving* body given one falls through the world.
fn collider_for(world: &mut World, shape: &str) -> Option<Collider> {
    match shape {
        "cube" => return Some(Collider::cuboid(1.0, 1.0, 1.0)),
        "sphere" => return Some(Collider::sphere(0.5)),
        "cylinder" => return Some(Collider::cylinder(0.5, 1.0)),
        "capsule" => return Some(Collider::capsule(0.5, 1.0)),
        // A plane mesh has no thickness at all, and a zero-height collider is one
        // a fast mover tunnels straight through. Thin, but not nothing.
        "plane" => return Some(Collider::cuboid(1.0, 0.02, 1.0)),
        _ => {}
    }

    let handle = mesh_handle(world, shape)?;
    let meshes = world.get_resource::<Assets<Mesh>>()?;
    trimesh_from_mesh(meshes.get(&handle)?)
}

/// Build a triangle-mesh collider out of a Bevy mesh.
///
/// Written out rather than calling avian's `Collider::trimesh_from_mesh`, which
/// is behind its `collider-from-mesh` feature and the engine does not enable it.
/// `renzora_physics` does exactly this for the same reason, and matching it
/// keeps one definition of "the collider for this mesh" in the project.
fn trimesh_from_mesh(mesh: &Mesh) -> Option<Collider> {
    use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return None;
    }
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(values) => values,
        _ => return None,
    };
    let vertices: Vec<Vec3> = positions
        .iter()
        .map(|p| Vec3::new(p[0], p[1], p[2]))
        .collect();
    let indices: Vec<[u32; 3]> = match mesh.indices()? {
        Indices::U32(idx) => idx.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
        Indices::U16(idx) => idx
            .chunks_exact(3)
            .map(|c| [c[0] as u32, c[1] as u32, c[2] as u32])
            .collect(),
    };
    if indices.is_empty() {
        return None;
    }
    Some(Collider::trimesh(vertices, indices))
}

/// Give an already-spawned subtree colliders.
///
/// For geometry built before colliders existed, or spawned with `collider:
/// false` and wanted solid after all. Walks the entity and its descendants, and
/// gives a collider to anything carrying a mesh that has not got one.
pub(super) fn make_solid(world: &mut World, args: &Value) -> ToolReply {
    let Some(root) = arg_entity(args, "entity") else {
        return ToolReply::Failed("make_solid needs an entity id".into());
    };
    if world.get_entity(root).is_err() {
        return ToolReply::Failed(format!("{} is not a live entity", entity_id(root)));
    }
    let fallback = args.get("shape").and_then(Value::as_str).unwrap_or("cube");

    let mut stack = vec![root];
    let mut targets = Vec::new();
    while let Some(at) = stack.pop() {
        if let Some(children) = world.get::<Children>(at) {
            stack.extend(children.iter());
        }
        // A mesh is the test for "is this geometry". A group entity is a
        // transform and nothing else, and giving it a collider would put an
        // invisible unit cube at the centre of the scene.
        if world.get::<Mesh3d>(at).is_some() && world.get::<Collider>(at).is_none() {
            // Each entity's own shape where it says what it is, so retrofitting a
            // mixed scene does not wrap its arches and ramps in boxes.
            let shape = world
                .get::<MeshPrimitive>(at)
                .map(|primitive| primitive.0.clone())
                .unwrap_or_else(|| fallback.to_string());
            targets.push((at, shape));
        }
    }

    // One collider per distinct shape, shared by every entity using it. Building
    // a trimesh per entity would rebuild the same triangle list for all ninety
    // walls in a row.
    let mut built: HashMap<String, Option<Collider>> = HashMap::default();
    let mut solid = Vec::new();
    for (entity, shape) in targets {
        let collider = match built.get(&shape) {
            Some(collider) => collider.clone(),
            None => {
                let built_now = collider_for(world, &shape);
                built.insert(shape, built_now.clone());
                built_now
            }
        };
        if let Some(collider) = collider {
            solid.push((entity, collider));
        }
    }
    let count = solid.len();
    if count > 0 {
        edits::make_solid(world, solid);
    }
    ToolReply::Text(format!("made {count} meshes solid under {}", entity_id(root)))
}

/// Spawn a light.
///
/// Separate from `spawn_entity` rather than another `shape`, because almost
/// nothing the two take is shared: a light has no mesh and no colour in the
/// material sense, and it is aimed rather than placed. Folding them together
/// would have meant a schema where half the fields are ignored depending on the
/// value of another.
///
/// It exists at all because a scene built entirely out of `spawn_entity` renders
/// black. That is not a hypothetical: the first market square built with these
/// tools had eighty meshes in it and nothing to light them.
pub(super) fn spawn_light(world: &mut World, args: &Value) -> ToolReply {
    let entity = match spawn_light_from_args(world, args) {
        Ok(entity) => entity,
        Err(why) => return ToolReply::Failed(why),
    };
    let name = name_of(world, entity);
    let kind = args.get("kind").and_then(Value::as_str).unwrap_or("directional");

    let again = args.clone();
    edits::record_spawn(world, entity, format!("spawn {name}"), move |world| {
        spawn_light_from_args(world, &again).ok()
    });

    ToolReply::Text(format!("spawned {kind} light {name} [{}]", entity_id(entity)))
}

fn spawn_light_from_args(world: &mut World, args: &Value) -> Result<Entity, String> {
    let Some(name) = args.get("name").and_then(Value::as_str) else {
        return Err("spawn_light needs a name".into());
    };
    // Checked before spawning, so a bad parent leaves nothing behind.
    let parent = arg_entity(args, "parent");
    if let Some(parent) = parent {
        if world.get_entity(parent).is_err() {
            return Err(format!("the parent {} does not exist", entity_id(parent)));
        }
    }
    let kind = args
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("directional");

    let color = arg_vec3(args, "color")
        .map(|c| Color::linear_rgb(c.x, c.y, c.z))
        .unwrap_or(Color::WHITE);

    let mut transform =
        Transform::from_translation(arg_vec3(args, "position").unwrap_or(Vec3::new(0.0, 8.0, 0.0)));
    // A directional light with no rotation points straight down -Z, which grazes
    // the ground and lights nothing. An afternoon sun angle is the useful default
    // and the one a caller who did not think about rotation meant.
    let euler = arg_vec3(args, "rotation").unwrap_or(match kind {
        "directional" => Vec3::new(-50.0, -35.0, 0.0),
        _ => Vec3::ZERO,
    });
    transform.rotation = Quat::from_euler(
        EulerRot::XYZ,
        euler.x.to_radians(),
        euler.y.to_radians(),
        euler.z.to_radians(),
    );

    let number = |key: &str, fallback: f32| {
        args.get(key)
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .unwrap_or(fallback)
    };
    let range = number("range", 20.0);

    let entity = match kind {
        "directional" => world
            .spawn((
                transform,
                DirectionalLight {
                    color,
                    // Lux. Overcast daylight, which reads as lit without blowing
                    // out a scene of untextured primitives the way full sun does.
                    illuminance: number("illuminance", 10_000.0),
                    shadow_maps_enabled: true,
                    ..default()
                },
            ))
            .id(),
        "point" => world
            .spawn((
                transform,
                PointLight {
                    color,
                    // Lumens, and Bevy's own default: a bright domestic bulb.
                    intensity: number("intensity", 1_000_000.0),
                    range,
                    shadow_maps_enabled: true,
                    ..default()
                },
            ))
            .id(),
        "spot" => world
            .spawn((
                transform,
                SpotLight {
                    color,
                    intensity: number("intensity", 1_000_000.0),
                    range,
                    outer_angle: number("outer_angle", 35.0).to_radians(),
                    inner_angle: number("inner_angle", 20.0).to_radians(),
                    shadow_maps_enabled: true,
                    ..default()
                },
            ))
            .id(),
        other => {
            return Err(format!(
                "unknown light kind: {other}; use directional, point or spot"
            ))
        }
    };
    unique_name(world, entity, name);

    if let Some(parent) = parent {
        world.entity_mut(entity).insert(ChildOf(parent));
    }

    Ok(entity)
}

pub(super) fn despawn_entity(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("despawn_entity needs an entity id".into());
    };
    let Ok(entity_mut) = world.get_entity_mut(entity) else {
        return ToolReply::Failed(format!("{} is not a live entity", entity_id(entity)));
    };
    // Despawn takes the children with it: `ChildOf` is a relationship, and Bevy
    // despawns a relationship's dependents with their target.
    entity_mut.despawn();
    ToolReply::Text(format!("despawned {}", entity_id(entity)))
}

pub(super) fn set_transform(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("set_transform needs an entity id".into());
    };
    let Some(mut transform) = world.get::<Transform>(entity).copied() else {
        return ToolReply::Failed(format!(
            "{} has no Transform (or is not live)",
            entity_id(entity)
        ));
    };

    let mut changed: Vec<&str> = Vec::new();
    if let Some(position) = arg_vec3(args, "position") {
        transform.translation = position;
        changed.push("position");
    }
    if let Some(euler) = arg_vec3(args, "rotation") {
        transform.rotation = Quat::from_euler(
            EulerRot::XYZ,
            euler.x.to_radians(),
            euler.y.to_radians(),
            euler.z.to_radians(),
        );
        changed.push("rotation");
    }
    if let Some(scale) = arg_vec3(args, "scale") {
        transform.scale = scale;
        changed.push("scale");
    }

    if changed.is_empty() {
        return ToolReply::Failed("set_transform was given nothing to change".into());
    }
    edits::set_transform(world, entity, transform);
    ToolReply::Text(format!(
        "set {} on {}",
        changed.join(", "),
        entity_id(entity)
    ))
}

pub(super) fn rename_entity(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("rename_entity needs an entity id".into());
    };
    let Some(name) = args.get("name").and_then(Value::as_str) else {
        return ToolReply::Failed("rename_entity needs a name".into());
    };
    if world.get_entity(entity).is_err() {
        return ToolReply::Failed(format!("{} is not a live entity", entity_id(entity)));
    }
    let unique = unique_entity_name(world, name, entity);
    edits::rename(world, entity, unique.clone());
    ToolReply::Text(format!("renamed {} to {unique}", entity_id(entity)))
}

/// Move an entity under a new parent, or out to the scene root.
///
/// A typed tool rather than `add_component ChildOf`, because `ChildOf` cannot be
/// registered for reflection: Bevy's own source carries a TODO saying so, since
/// the type has no meaningful default (a parent has to be a real entity, and
/// reflection builds a value before it knows one).
pub(super) fn reparent_entity(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("reparent_entity needs an entity id".into());
    };
    if world.get_entity(entity).is_err() {
        return ToolReply::Failed(format!("{} is not a live entity", entity_id(entity)));
    }

    let Some(parent) = arg_entity(args, "parent") else {
        // No parent given means "to the root", which is a real request and the
        // only way back out of a hierarchy.
        edits::reparent(world, entity, None);
        return ToolReply::Text(format!("moved {} to the scene root", entity_id(entity)));
    };
    if world.get_entity(parent).is_err() {
        return ToolReply::Failed(format!("{} is not a live entity", entity_id(parent)));
    }
    if parent == entity || is_descendant(world, parent, entity) {
        // Bevy would happily build the cycle and then the transform propagation
        // would recurse until the stack ran out.
        return ToolReply::Failed(
            "that would put an entity inside itself or inside its own descendant".into(),
        );
    }

    edits::reparent(world, entity, Some(parent));
    ToolReply::Text(format!(
        "moved {} under {}",
        entity_id(entity),
        entity_id(parent)
    ))
}

fn is_descendant(world: &World, maybe_child: Entity, ancestor: Entity) -> bool {
    let mut at = world.get::<ChildOf>(maybe_child).map(ChildOf::parent);
    while let Some(current) = at {
        if current == ancestor {
            return true;
        }
        at = world.get::<ChildOf>(current).map(ChildOf::parent);
    }
    false
}

/// Copy an entity, its components and its whole subtree.
///
/// `linked_cloning` is what takes the children with it. Without it a duplicated
/// stall would be one counter with no legs, posts or goods, which is never what
/// "duplicate" means to the person asking.
pub(super) fn duplicate_entity(world: &mut World, args: &Value) -> ToolReply {
    let Some(entity) = arg_entity(args, "entity") else {
        return ToolReply::Failed("duplicate_entity needs an entity id".into());
    };
    if world.get_entity(entity).is_err() {
        return ToolReply::Failed(format!("{} is not a live entity", entity_id(entity)));
    }

    let rename = args.get("name").and_then(Value::as_str).map(str::to_string);
    let offset = arg_vec3(args, "offset");
    let copy = duplicate_once(world, entity, rename.as_deref(), offset);

    // Recorded so Ctrl+Z removes the copy. A redo clones the source again,
    // which is the same thing the first call did, as long as it is still there.
    let label = format!("duplicate {}", name_of(world, entity));
    edits::record_spawn(world, copy, label, move |world| {
        world.get_entity(entity).ok()?;
        Some(duplicate_once(world, entity, rename.as_deref(), offset))
    });

    ToolReply::Text(format!(
        "duplicated {} as {} [{}]",
        entity_id(entity),
        name_of(world, copy),
        entity_id(copy)
    ))
}

/// One copy, named uniquely and moved by `offset`.
fn duplicate_once(world: &mut World, entity: Entity, name: Option<&str>, offset: Option<Vec3>) -> Entity {
    let copy = clone_subtree(world, entity);
    let wanted = name.map(str::to_string).unwrap_or_else(|| name_of(world, entity));
    unique_name(world, copy, &wanted);
    if let (Some(offset), Some(mut transform)) = (offset, world.get_mut::<Transform>(copy)) {
        transform.translation += offset;
    }
    copy
}

/// Clone an entity and everything under it.
///
/// Linked cloning is what carries the children across, and it follows EVERY
/// relationship marked `linked_spawn`, not only `Children`. Avian's
/// `RigidBodyColliders` is one, and a body that is its own collider (every solid
/// `spawn_entity` shape) lists itself in it. The cloner then queued the entity
/// as its own child, cloned that, queued it again, and the editor hung with
/// memory climbing at about 65 MB a second. Both halves of the relationship are
/// left out: inserting `Collider` on the copy rebuilds them, exactly as it does
/// on a fresh spawn.
fn clone_subtree(world: &mut World, entity: Entity) -> Entity {
    world
        .entity_mut(entity)
        .clone_and_spawn_with_opt_out(|builder| {
            builder
                .linked_cloning(true)
                .deny::<(RigidBodyColliders, ColliderOf)>();
        })
}
