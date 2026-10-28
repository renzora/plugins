//! The 2D world on rapier2d, the sprite-world sibling of [`crate::world3d`].
//!
//! The engine resolves every 2D collider into a 2D shape before it gets here,
//! so this only maps rectangles, circles and capsules onto rapier2d, and reads
//! the XY of every vector and the turn about Z of every rotation.

use std::collections::HashMap;

use bevy::prelude::*;
use rapier2d::prelude as rp;
use renzora::physics::{
    physics_backend_selected, AppPhysicsBackendExt, BodyVelocity, Caps, CastBatch, CastHit,
    CastShape, CollisionLayers, Contact, JointKind, OverlapBatch, PhysicsBackend, PhysicsBodyData,
    PhysicsBodyType, PhysicsCommand, PhysicsCommands, PhysicsContacts, PhysicsDim, PhysicsJoint,
    PhysicsSet, PhysicsSettings, ResolvedCollider, ResolvedShape, RuntimePhysics, RuntimePhysics2d,
};

use crate::{entity_bits, entity_from, world_transform};

/// The name this backend registers under, and the one `project.toml` uses.
pub const NAME: &str = "rapier2d";

pub(crate) fn build(app: &mut App) {
    app.init_resource::<Rapier2d>();
    app.add_physics_backend(Rapier2dBackend);

    let active = physics_backend_selected(NAME, PhysicsDim::D2);
    app.add_systems(Update, sync.in_set(PhysicsSet::Sync).run_if(active.clone()));
    app.add_systems(Update, write_state.in_set(PhysicsSet::Writeback).run_if(active.clone()));
    app.add_systems(FixedPostUpdate, (step, write_transforms).chain().run_if(active.clone()));
    app.add_systems(PostUpdate, apply_commands.in_set(PhysicsSet::Commands).run_if(active));
}

pub struct Rapier2dBackend;

impl PhysicsBackend for Rapier2dBackend {
    fn name(&self) -> &str {
        NAME
    }

    fn caps(&self) -> Caps {
        Caps::DIM_2D.union(Caps::JOINTS).union(Caps::CCD)
    }

    fn cast(&self, world: &World, batch: &mut CastBatch) {
        if let Some(rapier) = world.get_resource::<Rapier2d>() {
            cast(rapier, world, batch);
        }
    }

    fn overlap(&self, world: &World, batch: &mut OverlapBatch) {
        if let Some(rapier) = world.get_resource::<Rapier2d>() {
            overlap(rapier, batch);
        }
    }

    fn detach(&self, world: &mut World, dim: PhysicsDim) {
        if dim == PhysicsDim::D2 {
            world.insert_resource(Rapier2d::default());
        }
    }
}

/// Rapier's 2D world plus the maps between it and the ECS. See
/// `world3d::Rapier3d` for what each field is for.
#[derive(Resource, Default)]
pub struct Rapier2d {
    pub world: rp::PhysicsWorld,
    bodies: HashMap<Entity, rp::RigidBodyHandle>,
    colliders: HashMap<Entity, rp::ColliderHandle>,
    joints: HashMap<Entity, rp::ImpulseJointHandle>,
    pending_joints: Vec<Entity>,
    written: HashMap<Entity, (Vec3, Quat)>,
    forced: Vec<rp::RigidBodyHandle>,
}

impl Rapier2d {
    fn remove_body(&mut self, entity: Entity) {
        if let Some(handle) = self.bodies.remove(&entity) {
            self.world.remove_body_with_colliders(handle, false);
        }
        self.written.remove(&entity);
    }

    fn remove_collider(&mut self, entity: Entity) {
        if let Some(handle) = self.colliders.remove(&entity) {
            self.world.remove_collider(handle);
        }
    }

    fn remove_joint(&mut self, entity: Entity) {
        if let Some(handle) = self.joints.remove(&entity) {
            self.world.remove_impulse_joint(handle);
        }
    }

    fn entity_of(&self, collider: rp::ColliderHandle) -> Option<Entity> {
        self.world.colliders.get(collider).map(|c| entity_from(c.user_data))
    }
}

// ── Math ─────────────────────────────────────────────────────────────────

fn to_rapier(v: Vec3) -> rp::Vector {
    rp::Vector::new(v.x, v.y)
}

fn to_bevy(v: rp::Vector) -> Vec2 {
    Vec2::new(v.x, v.y)
}

/// The plane's one rotation: the turn about Z.
fn z_angle(rotation: Quat) -> f32 {
    rotation.to_euler(EulerRot::ZYX).0
}

fn pose(translation: Vec3, rotation: Quat) -> rp::Pose {
    rp::Pose::new(to_rapier(translation), z_angle(rotation))
}

fn groups(layers: CollisionLayers) -> rp::InteractionGroups {
    rp::InteractionGroups::new(
        rp::Group::from_bits_retain(layers.memberships),
        rp::Group::from_bits_retain(layers.filters),
        rp::InteractionTestMode::And,
    )
}

// ── Building ─────────────────────────────────────────────────────────────

fn shape_for(shape: &ResolvedShape, scale: Vec3) -> Option<rp::SharedShape> {
    let s = scale.abs();
    Some(match shape {
        ResolvedShape::Rectangle { half_extents } => {
            rp::SharedShape::cuboid(half_extents.x * s.x, half_extents.y * s.y)
        }
        ResolvedShape::Circle { radius } => rp::SharedShape::ball(radius * s.x.max(s.y)),
        ResolvedShape::Capsule2d { radius, half_height } => {
            rp::SharedShape::capsule_y(half_height * s.y, radius * s.x)
        }
        // The 3D world never reaches this backend.
        ResolvedShape::Cuboid { .. }
        | ResolvedShape::Sphere { .. }
        | ResolvedShape::Capsule { .. }
        | ResolvedShape::Cylinder { .. }
        | ResolvedShape::TriMesh(_) => return None,
    })
}

fn body_type(body: &PhysicsBodyData) -> rp::RigidBodyType {
    match body.body_type {
        PhysicsBodyType::RigidBody => rp::RigidBodyType::Dynamic,
        PhysicsBodyType::StaticBody => rp::RigidBodyType::Fixed,
        PhysicsBodyType::KinematicBody => rp::RigidBodyType::KinematicPositionBased,
    }
}

/// 2D has one rotation; the shared data's Z-rotation lock is the one that maps
/// onto it, as on avian2d.
fn locked_axes(body: &PhysicsBodyData) -> rp::LockedAxes {
    let mut locked = rp::LockedAxes::empty();
    for (on, axis) in [
        (body.lock_translation_x, rp::LockedAxes::TRANSLATION_LOCKED_X),
        (body.lock_translation_y, rp::LockedAxes::TRANSLATION_LOCKED_Y),
        (body.lock_rotation_z, rp::LockedAxes::ROTATION_LOCKED),
    ] {
        if on {
            locked |= axis;
        }
    }
    locked
}

fn owning_body(
    entity: Entity,
    rapier: &Rapier2d,
    parents: &Query<&ChildOf>,
) -> Option<(Entity, rp::RigidBodyHandle)> {
    let mut e = entity;
    loop {
        if let Some(handle) = rapier.bodies.get(&e) {
            return Some((e, *handle));
        }
        e = parents.get(e).ok()?.parent();
    }
}

fn build_collider(
    entity: Entity,
    resolved: &ResolvedCollider,
    rapier: &mut Rapier2d,
    hierarchy: &Query<(&Transform, Option<&ChildOf>)>,
    parents: &Query<&ChildOf>,
    body_data: &Query<&PhysicsBodyData>,
) {
    rapier.remove_collider(entity);
    let world = world_transform(entity, hierarchy);
    let Some(shape) = shape_for(&resolved.shape, world.scale) else {
        return;
    };
    let mut builder = rp::ColliderBuilder::new(shape)
        .friction(resolved.friction)
        .restitution(resolved.restitution)
        .sensor(resolved.sensor)
        .collision_groups(groups(resolved.layers))
        .user_data(entity_bits(entity));
    let offset = resolved.offset * world.scale;

    let handle = match owning_body(entity, rapier, parents) {
        Some((owner, body)) => {
            let relative = if owner == entity {
                pose(offset, resolved.rotation)
            } else {
                let owner_world = world_transform(owner, hierarchy);
                let inverse = owner_world.rotation.inverse();
                let local = inverse * (world.translation - owner_world.translation);
                let rotation = inverse * world.rotation;
                pose(local + rotation * offset, rotation * resolved.rotation)
            };
            if owner == entity {
                if let Ok(data) = body_data.get(entity) {
                    builder = builder.mass(data.mass);
                }
            }
            rapier.world.insert_collider(builder.position(relative), Some(body))
        }
        None => {
            let at = world.translation + world.rotation * offset;
            rapier
                .world
                .insert_collider(builder.position(pose(at, world.rotation * resolved.rotation)), None)
        }
    };
    rapier.colliders.insert(entity, handle);
}

/// The rapier2d joint a [`PhysicsJoint`] describes; `None` for a spherical
/// joint, which has nothing to turn about in a plane but Z. Rigid, as in 3D:
/// see `world3d::joint_for` for why `compliance` is not carried over.
fn joint_for(joint: &PhysicsJoint) -> Option<rp::GenericJoint> {
    let (a1, a2) = (to_rapier(joint.anchor1), to_rapier(joint.anchor2));
    let mut built: rp::GenericJoint = match joint.kind {
        JointKind::Fixed => rp::FixedJointBuilder::new().local_anchor1(a1).local_anchor2(a2).into(),
        JointKind::Revolute { limits, .. } => {
            let mut b = rp::RevoluteJointBuilder::new().local_anchor1(a1).local_anchor2(a2);
            if let Some((min, max)) = limits {
                b = b.limits([min, max]);
            }
            b.into()
        }
        JointKind::Prismatic { axis, limits } => {
            let axis = to_rapier(axis.truncate().normalize_or(Vec2::X).extend(0.0));
            let mut b = rp::PrismaticJointBuilder::new(axis).local_anchor1(a1).local_anchor2(a2);
            if let Some((min, max)) = limits {
                b = b.limits([min, max]);
            }
            b.into()
        }
        JointKind::Distance { max, .. } => {
            rp::RopeJointBuilder::new(max).local_anchor1(a1).local_anchor2(a2).into()
        }
        JointKind::Spherical { .. } => return None,
    };
    built.set_contacts_enabled(joint.collide_connected);
    Some(built)
}

/// False when either body is not built yet. A joint this world cannot build at
/// all is reported once and counts as done.
fn build_joint(entity: Entity, joint: &PhysicsJoint, rapier: &mut Rapier2d) -> bool {
    rapier.remove_joint(entity);
    let (Some(b1), Some(b2)) = (rapier.bodies.get(&joint.body1).copied(), rapier.bodies.get(&joint.body2).copied())
    else {
        return false;
    };
    let Some(built) = joint_for(joint) else {
        warn!("[rapier2d] a spherical joint has no meaning in 2D; use a revolute joint there");
        return true;
    };
    let handle = rapier.world.insert_impulse_joint(b1, b2, built);
    rapier.joints.insert(entity, handle);
    true
}

// ── Systems ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync(
    mut rapier: ResMut<Rapier2d>,
    // One parameter for the four, to stay inside Bevy's sixteen.
    (mut removed_runtime, mut removed_colliders, mut removed_joints, mut removed_bodies): (
        RemovedComponents<RuntimePhysics>,
        RemovedComponents<ResolvedCollider>,
        RemovedComponents<PhysicsJoint>,
        RemovedComponents<PhysicsBodyData>,
    ),
    live: Query<(), With<RuntimePhysics>>,
    has_collider: Query<(), With<ResolvedCollider>>,
    new_bodies: Query<
        (Entity, &PhysicsBodyData),
        (
            With<RuntimePhysics>,
            With<RuntimePhysics2d>,
            Or<(Added<RuntimePhysics>, Changed<PhysicsBodyData>)>,
        ),
    >,
    changed_colliders: Query<
        Entity,
        (With<RuntimePhysics>, With<RuntimePhysics2d>, Changed<ResolvedCollider>),
    >,
    changed_joints: Query<Entity, Changed<PhysicsJoint>>,
    all_joints: Query<&PhysicsJoint>,
    dims: Query<Has<RuntimePhysics2d>>,
    moved: Query<(Entity, &Transform), (With<RuntimePhysics>, With<RuntimePhysics2d>, Changed<Transform>)>,
    colliders: Query<&ResolvedCollider>,
    hierarchy: Query<(&Transform, Option<&ChildOf>)>,
    parents: Query<&ChildOf>,
    body_data: Query<&PhysicsBodyData>,
) {
    let rapier = &mut *rapier;

    for entity in removed_runtime.read() {
        if !live.contains(entity) {
            rapier.remove_body(entity);
            rapier.remove_collider(entity);
        }
    }
    for entity in removed_colliders.read() {
        if !has_collider.contains(entity) {
            rapier.remove_collider(entity);
        }
    }
    for entity in removed_joints.read() {
        if all_joints.get(entity).is_err() {
            rapier.remove_joint(entity);
            rapier.pending_joints.retain(|e| *e != entity);
        }
    }

    let mut rebuilt = Vec::new();
    // See `world3d::sync`: the body goes, its collider stays as a fixed one.
    for entity in removed_bodies.read() {
        if live.contains(entity) && body_data.get(entity).is_err() && rapier.bodies.contains_key(&entity) {
            rapier.remove_body(entity);
            rebuilt.push(entity);
        }
    }
    let mut new_body = false;
    for (entity, data) in &new_bodies {
        match rapier.bodies.get(&entity).copied() {
            Some(handle) => {
                let Some(body) = rapier.world.bodies.get_mut(handle) else { continue };
                body.set_body_type(body_type(data), true);
                body.set_gravity_scale(data.gravity_scale, true);
                body.set_linear_damping(data.linear_damping);
                body.set_angular_damping(data.angular_damping);
                body.set_locked_axes(locked_axes(data), true);
                body.enable_ccd(data.ccd);
                rebuilt.push(entity);
            }
            None => {
                let world = world_transform(entity, &hierarchy);
                let body = rp::RigidBodyBuilder::new(body_type(data))
                    .pose(pose(world.translation, world.rotation))
                    .gravity_scale(data.gravity_scale)
                    .linear_damping(data.linear_damping)
                    .angular_damping(data.angular_damping)
                    .locked_axes(locked_axes(data))
                    .ccd_enabled(data.ccd)
                    .user_data(entity_bits(entity))
                    .build();
                let handle = rapier.world.insert_body(body);
                rapier.bodies.insert(entity, handle);
                new_body = true;
                if rapier.colliders.contains_key(&entity) {
                    rebuilt.push(entity);
                }
            }
        }
    }

    for entity in changed_colliders.iter().chain(rebuilt) {
        if let Ok(resolved) = colliders.get(entity) {
            build_collider(entity, resolved, rapier, &hierarchy, &parents, &body_data);
        }
    }

    if new_body {
        for (entity, handle) in rapier.joints.clone() {
            if rapier.world.impulse_joints.get(handle).is_none() && !rapier.pending_joints.contains(&entity) {
                rapier.pending_joints.push(entity);
            }
        }
    }
    let mut todo: Vec<Entity> = std::mem::take(&mut rapier.pending_joints);
    for entity in &changed_joints {
        if !todo.contains(&entity) {
            todo.push(entity);
        }
    }
    for entity in todo {
        let Ok(joint) = all_joints.get(entity) else { continue };
        if dims.get(joint.body1) != Ok(true) {
            continue;
        }
        if !build_joint(entity, joint, rapier) {
            rapier.pending_joints.push(entity);
        }
    }

    for (entity, local) in &moved {
        if rapier.written.get(&entity) == Some(&(local.translation, local.rotation)) {
            continue;
        }
        let world = world_transform(entity, &hierarchy);
        let target = pose(world.translation, world.rotation);
        if let Some(handle) = rapier.bodies.get(&entity).copied() {
            if let Some(body) = rapier.world.bodies.get_mut(handle) {
                if body.is_kinematic() {
                    body.set_next_kinematic_position(target);
                } else {
                    body.set_position(target, true);
                }
            }
        } else if let Some(handle) = rapier.colliders.get(&entity).copied() {
            let standalone = rapier.world.colliders.get(handle).is_some_and(|c| c.parent().is_none());
            if standalone {
                if let (Some(c), Ok(resolved)) = (rapier.world.colliders.get_mut(handle), colliders.get(entity)) {
                    let offset = resolved.offset * world.scale;
                    c.set_position(pose(
                        world.translation + world.rotation * offset,
                        world.rotation * resolved.rotation,
                    ));
                }
            }
        }
    }
}

fn step(mut rapier: ResMut<Rapier2d>, settings: Res<PhysicsSettings>, time: Res<Time<Fixed>>) {
    if settings.paused {
        return;
    }
    let dt = time.delta_secs() * settings.time_scale;
    if dt <= 0.0 {
        return;
    }
    let world = &mut rapier.world;
    world.integration_parameters.dt = dt;
    world.integration_parameters.num_solver_iterations = settings.substeps.max(1) as usize;
    world.gravity = to_rapier(settings.gravity);
    world.step();
}

/// Dynamic bodies back into their `Transform`s. The plane's depth (`z`) is
/// draw order, not physics, so it is kept as it was.
fn write_transforms(
    mut rapier: ResMut<Rapier2d>,
    mut transforms: Query<(&mut Transform, Option<&ChildOf>)>,
    globals: Query<&GlobalTransform>,
) {
    let rapier = &mut *rapier;
    for (entity, handle) in &rapier.bodies {
        let Some(body) = rapier.world.bodies.get(*handle) else { continue };
        if !body.is_dynamic() {
            continue;
        }
        let Ok((mut transform, parent)) = transforms.get_mut(*entity) else { continue };
        let mut translation = to_bevy(body.translation()).extend(0.0);
        let mut rotation = Quat::from_rotation_z(body.rotation().angle());
        if let Some(parent) = parent.map(|p| p.parent()) {
            let simulated = rapier
                .bodies
                .get(&parent)
                .and_then(|h| rapier.world.bodies.get(*h))
                .map(|b| (to_bevy(b.translation()).extend(0.0), Quat::from_rotation_z(b.rotation().angle())));
            let from_global = || {
                globals.get(parent).ok().map(|g| {
                    let (_, r, t) = g.to_scale_rotation_translation();
                    (t.with_z(0.0), r)
                })
            };
            if let Some((parent_translation, parent_rotation)) = simulated.or_else(from_global) {
                let inverse = parent_rotation.inverse();
                translation = inverse * (translation - parent_translation);
                rotation = inverse * rotation;
            }
        }
        let translation = translation.with_z(transform.translation.z);
        if transform.translation != translation || transform.rotation != rotation {
            transform.translation = translation;
            transform.rotation = rotation;
        }
        rapier.written.insert(*entity, (translation, rotation));
    }
}

#[allow(clippy::type_complexity)]
fn write_state(
    rapier: Res<Rapier2d>,
    mut velocities: Query<(Entity, &mut BodyVelocity), With<RuntimePhysics2d>>,
    mut contacts: Query<(Entity, &mut PhysicsContacts, Option<&GlobalTransform>), With<RuntimePhysics2d>>,
) {
    for (entity, mut velocity) in &mut velocities {
        let v = rapier
            .bodies
            .get(&entity)
            .and_then(|h| rapier.world.bodies.get(*h))
            .map(|b| BodyVelocity { linear: to_bevy(b.linvel()).extend(0.0), angular: Vec3::Z * b.angvel() })
            .unwrap_or_default();
        velocity.set_if_neq(v);
    }
    for (entity, mut touching, transform) in &mut contacts {
        let Some(handle) = rapier.colliders.get(&entity).copied() else {
            if !touching.0.is_empty() {
                touching.0.clear();
            }
            continue;
        };
        let here = transform.map(|t| t.translation()).unwrap_or_default();
        let mut current: Vec<Contact> = Vec::new();
        for pair in rapier.world.contact_pairs_with(handle).filter(|p| p.has_any_active_contact()) {
            let first = pair.collider1 == handle;
            let Some(other) = rapier.entity_of(if first { pair.collider2 } else { pair.collider1 }) else {
                continue;
            };
            let (point, normal) = match pair.find_deepest_contact() {
                Some((manifold, contact)) => {
                    let at = rapier
                        .world
                        .colliders
                        .get(pair.collider1)
                        .map(|c| to_bevy(c.position().transform_point(contact.local_p1)).extend(here.z))
                        .unwrap_or(here);
                    let n = to_bevy(manifold.data.normal).extend(0.0);
                    (at, if first { -n } else { n })
                }
                None => (here, Vec3::ZERO),
            };
            current.push(Contact { entity: other, point, normal, impulse: pair.total_impulse_magnitude() });
        }
        for (a, _, b, _, intersecting) in rapier.world.intersection_pairs_with(handle) {
            if !intersecting {
                continue;
            }
            if let Some(other) = rapier.entity_of(if a == handle { b } else { a }) {
                current.push(Contact { entity: other, point: here, normal: Vec3::ZERO, impulse: 0.0 });
            }
        }
        if touching.0 != current {
            touching.0 = current;
        }
    }
}

/// The 2D twin of `world3d::apply_commands`: vectors lose their `z`, spins
/// keep only theirs.
fn apply_commands(
    mut rapier: ResMut<Rapier2d>,
    queue: Res<PhysicsCommands>,
    dims: Query<Has<RuntimePhysics2d>, With<RuntimePhysics>>,
) {
    let rapier = &mut *rapier;
    for handle in std::mem::take(&mut rapier.forced) {
        if let Some(body) = rapier.world.bodies.get_mut(handle) {
            body.reset_forces(false);
            body.reset_torques(false);
        }
    }
    for command in &queue.0 {
        let entity = command.entity();
        if let PhysicsCommand::Strip { entity } = *command {
            rapier.remove_body(entity);
            rapier.remove_collider(entity);
            continue;
        }
        if dims.get(entity) != Ok(true) {
            continue;
        }
        let Some(handle) = rapier.bodies.get(&entity).copied() else { continue };
        let Some(body) = rapier.world.bodies.get_mut(handle) else { continue };
        match *command {
            PhysicsCommand::AddForce { force, .. } => {
                body.add_force(to_rapier(force), true);
                rapier.forced.push(handle);
            }
            PhysicsCommand::AddTorque { torque, .. } => {
                body.add_torque(torque.z, true);
                rapier.forced.push(handle);
            }
            PhysicsCommand::AddImpulse { impulse, .. } => body.apply_impulse(to_rapier(impulse), true),
            PhysicsCommand::AddAngularImpulse { impulse, .. } => body.apply_torque_impulse(impulse.z, true),
            PhysicsCommand::SetLinearVelocity { velocity, .. } => body.set_linvel(to_rapier(velocity), true),
            PhysicsCommand::SetAngularVelocity { velocity, .. } => body.set_angvel(velocity.z, true),
            PhysicsCommand::Strip { .. } => {}
        }
    }
}

// ── Queries ──────────────────────────────────────────────────────────────

fn cast(rapier: &Rapier2d, world: &World, batch: &mut CastBatch) {
    let CastBatch { casts, hits } = batch;
    for (cast, hit) in casts.iter().zip(hits.iter_mut()) {
        if cast.dim != PhysicsDim::D2 {
            continue;
        }
        let Ok(dir) = Dir2::new(cast.direction.truncate()) else { continue };
        let mask = cast.collides_with;
        let keep = |_: rp::ColliderHandle, c: &rp::Collider| {
            (cast.include_sensors || !c.is_sensor())
                && c.collision_groups().memberships.bits() & mask != 0
                && !cast.exclude.contains(&entity_from(c.user_data))
        };
        let filter = rp::QueryFilter::default().predicate(&keep);
        let direction = rp::Vector::new(dir.x, dir.y);
        let mut at = pose(cast.origin, cast.rotation);
        let shape = match &cast.shape {
            CastShape::Ray => {
                let ray = rp::Ray::new(to_rapier(cast.origin), direction);
                let solid = !cast.ignore_origin_penetration;
                if let Some((h, i)) = rapier.world.cast_ray_and_get_normal(&ray, cast.max_distance, solid, filter) {
                    if let Some(entity) = rapier.entity_of(h) {
                        *hit = Some(CastHit {
                            distance: i.time_of_impact,
                            point: (cast.origin.truncate() + *dir * i.time_of_impact).extend(cast.origin.z),
                            normal: to_bevy(i.normal).extend(0.0),
                            entity,
                        });
                    }
                }
                continue;
            }
            CastShape::Shape(shape) => match shape_for(shape, Vec3::ONE) {
                Some(s) => s,
                None => continue,
            },
            CastShape::Collider(entity) => {
                let Some(resolved) = world.get::<ResolvedCollider>(*entity) else { continue };
                let scale = world.get::<GlobalTransform>(*entity).map(|g| g.scale()).unwrap_or(Vec3::ONE);
                at = pose(
                    cast.origin + cast.rotation * (resolved.offset * scale),
                    cast.rotation * resolved.rotation,
                );
                let built = rapier.colliders.get(entity).and_then(|h| rapier.world.colliders.get(*h));
                match built {
                    Some(c) => c.shared_shape().clone(),
                    None => match shape_for(&resolved.shape, scale) {
                        Some(s) => s,
                        None => continue,
                    },
                }
            }
        };
        let options = rapier2d::parry::query::ShapeCastOptions {
            max_time_of_impact: cast.max_distance,
            target_distance: 0.0,
            stop_at_penetration: !cast.ignore_origin_penetration,
            compute_impact_geometry_on_penetration: true,
        };
        if let Some((h, s)) = rapier.world.cast_shape(&at, direction, &*shape, options, filter) {
            if let Some(entity) = rapier.entity_of(h) {
                *hit = Some(CastHit {
                    distance: s.time_of_impact,
                    point: to_bevy(s.witness1).extend(cast.origin.z),
                    normal: to_bevy(s.normal1).extend(0.0),
                    entity,
                });
            }
        }
    }
}

fn overlap(rapier: &Rapier2d, batch: &mut OverlapBatch) {
    let OverlapBatch { queries, hits } = batch;
    for (query, found) in queries.iter().zip(hits.iter_mut()) {
        if query.dim != PhysicsDim::D2 {
            continue;
        }
        let Some(shape) = shape_for(&query.shape, Vec3::ONE) else { continue };
        let mask = query.collides_with;
        let keep = |_: rp::ColliderHandle, c: &rp::Collider| {
            (query.include_sensors || !c.is_sensor())
                && c.collision_groups().memberships.bits() & mask != 0
                && !query.exclude.contains(&entity_from(c.user_data))
        };
        let filter = rp::QueryFilter::default().predicate(&keep);
        for (handle, _) in rapier.world.intersect_shape(pose(query.position, query.rotation), &*shape, filter) {
            if let Some(entity) = rapier.entity_of(handle) {
                found.push(entity);
            }
        }
    }
}
