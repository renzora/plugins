//! The 3D world on rapier3d.

use std::collections::HashMap;

use bevy::prelude::*;
use rapier3d::prelude as rp;
use renzora::physics::{
    physics_backend_selected, AppPhysicsBackendExt, BodyVelocity, Caps, CastBatch, CastHit,
    CastShape, CollisionLayers, Contact, JointKind, OverlapBatch, PhysicsBackend, PhysicsBodyData,
    PhysicsBodyType, PhysicsCommand, PhysicsCommands, PhysicsContacts, PhysicsDim, PhysicsJoint,
    PhysicsSet, PhysicsSettings, ResolvedCollider, ResolvedShape, RuntimePhysics, RuntimePhysics2d,
};

use crate::{entity_bits, entity_from, world_transform};

/// The name this backend registers under, and the one `project.toml` uses.
pub const NAME: &str = "rapier3d";

pub(crate) fn build(app: &mut App) {
    app.init_resource::<Rapier3d>();
    app.add_physics_backend(Rapier3dBackend);

    let active = physics_backend_selected(NAME, PhysicsDim::D3);
    app.add_systems(Update, sync.in_set(PhysicsSet::Sync).run_if(active.clone()));
    app.add_systems(Update, write_state.in_set(PhysicsSet::Writeback).run_if(active.clone()));
    // Stepped where avian steps, on the fixed clock, so a scene behaves the
    // same per second on either backend.
    app.add_systems(FixedPostUpdate, (step, write_transforms).chain().run_if(active.clone()));
    app.add_systems(PostUpdate, apply_commands.in_set(PhysicsSet::Commands).run_if(active));
}

pub struct Rapier3dBackend;

impl PhysicsBackend for Rapier3dBackend {
    fn name(&self) -> &str {
        NAME
    }

    fn caps(&self) -> Caps {
        Caps::DIM_3D.union(Caps::TRIMESH).union(Caps::JOINTS).union(Caps::CCD)
    }

    fn cast(&self, world: &World, batch: &mut CastBatch) {
        if let Some(rapier) = world.get_resource::<Rapier3d>() {
            cast(rapier, world, batch);
        }
    }

    fn overlap(&self, world: &World, batch: &mut OverlapBatch) {
        if let Some(rapier) = world.get_resource::<Rapier3d>() {
            overlap(rapier, batch);
        }
    }

    /// The whole world goes: every body, collider, joint and handle. The engine
    /// re-wires each entity for whichever backend comes next.
    fn detach(&self, world: &mut World, dim: PhysicsDim) {
        if dim == PhysicsDim::D3 {
            world.insert_resource(Rapier3d::default());
        }
    }
}

/// Rapier's 3D world plus the maps between it and the ECS.
#[derive(Resource, Default)]
pub struct Rapier3d {
    pub world: rp::PhysicsWorld,
    bodies: HashMap<Entity, rp::RigidBodyHandle>,
    colliders: HashMap<Entity, rp::ColliderHandle>,
    joints: HashMap<Entity, rp::ImpulseJointHandle>,
    /// Joints waiting for a body that has not been built yet.
    pending_joints: Vec<Entity>,
    /// The local transform this backend last wrote to each dynamic body. A
    /// `Transform` that differs from it was moved by someone else (the editor,
    /// a script) and is a teleport; one that matches is our own write coming
    /// back as `Changed`.
    written: HashMap<Entity, (Vec3, Quat)>,
    /// Bodies holding a command force or torque, reset before the next frame's
    /// commands.
    forced: Vec<rp::RigidBodyHandle>,
}

impl Rapier3d {
    fn remove_body(&mut self, entity: Entity) {
        if let Some(handle) = self.bodies.remove(&entity) {
            // Keep attached colliders: a child's collider belongs to its own
            // entity and is removed when that entity says so. Joints on the
            // body go with it; `rebuild_joints_of` brings them back if it does.
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

// ── Math between Bevy's glam and rapier's ────────────────────────────────
//
// The same glam version, but a separate compilation inside a plugin build, so
// the types are distinct and cross by value.

fn to_rapier(v: Vec3) -> rp::Vector {
    rp::Vector::new(v.x, v.y, v.z)
}

fn to_bevy(v: rp::Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn rot_to_rapier(q: Quat) -> rp::Rotation {
    rp::Rotation::from_xyzw(q.x, q.y, q.z, q.w)
}

fn rot_to_bevy(q: rp::Rotation) -> Quat {
    Quat::from_xyzw(q.x, q.y, q.z, q.w)
}

fn pose(translation: Vec3, rotation: Quat) -> rp::Pose {
    rp::Pose::from_parts(to_rapier(translation), rot_to_rapier(rotation))
}

fn groups(layers: CollisionLayers) -> rp::InteractionGroups {
    rp::InteractionGroups::new(
        rp::Group::from_bits_retain(layers.memberships),
        rp::Group::from_bits_retain(layers.filters),
        rp::InteractionTestMode::And,
    )
}

// ── Building ─────────────────────────────────────────────────────────────

/// A rapier shape for a resolved collider, sized by the entity's world scale
/// the way avian scales its colliders.
fn shape_for(shape: &ResolvedShape, scale: Vec3) -> Option<rp::SharedShape> {
    let s = scale.abs();
    Some(match shape {
        ResolvedShape::Cuboid { half_extents } => {
            let h = *half_extents * s;
            rp::SharedShape::cuboid(h.x, h.y, h.z)
        }
        ResolvedShape::Sphere { radius } => rp::SharedShape::ball(radius * s.max_element()),
        ResolvedShape::Capsule { radius, half_height } => {
            rp::SharedShape::capsule_y(half_height * s.y, radius * s.x.max(s.z))
        }
        ResolvedShape::Cylinder { radius, half_height } => {
            rp::SharedShape::cylinder(half_height * s.y, radius * s.x.max(s.z))
        }
        ResolvedShape::TriMesh(mesh) => {
            let vertices = mesh.vertices.iter().map(|v| to_rapier(*v * scale)).collect();
            rp::SharedShape::trimesh(vertices, mesh.indices.clone()).ok()?
        }
        // The 2D world never reaches this backend.
        ResolvedShape::Rectangle { .. } | ResolvedShape::Circle { .. } | ResolvedShape::Capsule2d { .. } => {
            return None
        }
    })
}

fn body_type(body: &PhysicsBodyData) -> rp::RigidBodyType {
    match body.body_type {
        PhysicsBodyType::RigidBody => rp::RigidBodyType::Dynamic,
        PhysicsBodyType::StaticBody => rp::RigidBodyType::Fixed,
        PhysicsBodyType::KinematicBody => rp::RigidBodyType::KinematicPositionBased,
    }
}

fn locked_axes(body: &PhysicsBodyData) -> rp::LockedAxes {
    let mut locked = rp::LockedAxes::empty();
    for (on, axis) in [
        (body.lock_translation_x, rp::LockedAxes::TRANSLATION_LOCKED_X),
        (body.lock_translation_y, rp::LockedAxes::TRANSLATION_LOCKED_Y),
        (body.lock_translation_z, rp::LockedAxes::TRANSLATION_LOCKED_Z),
        (body.lock_rotation_x, rp::LockedAxes::ROTATION_LOCKED_X),
        (body.lock_rotation_y, rp::LockedAxes::ROTATION_LOCKED_Y),
        (body.lock_rotation_z, rp::LockedAxes::ROTATION_LOCKED_Z),
    ] {
        if on {
            locked |= axis;
        }
    }
    locked
}

/// The nearest body at or above `entity`, and which entity holds it.
fn owning_body(
    entity: Entity,
    rapier: &Rapier3d,
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

/// Build `entity`'s collider, attached to the body that owns it, if any.
///
/// A collider on the body's own entity carries the body's mass, so the body
/// weighs what `PhysicsBodyData.mass` says, as it does under avian. One on a
/// child adds its shape's own mass on top, which is the price of attaching it.
fn build_collider(
    entity: Entity,
    resolved: &ResolvedCollider,
    rapier: &mut Rapier3d,
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

/// The rapier joint a [`PhysicsJoint`] describes.
///
/// `compliance` is not carried over: rapier softens a joint with a spring
/// (stiffness and damping), not an XPBD compliance, and no one mapping between
/// them holds for every mass. A rapier joint is rigid.
fn joint_for(joint: &PhysicsJoint) -> rp::GenericJoint {
    let (a1, a2) = (to_rapier(joint.anchor1), to_rapier(joint.anchor2));
    let mut built: rp::GenericJoint = match joint.kind {
        JointKind::Fixed => rp::FixedJointBuilder::new().local_anchor1(a1).local_anchor2(a2).into(),
        JointKind::Spherical { twist_axis, swing_limit, twist_limit } => {
            // The frame's Y is the twist axis; X and Z are the two swings, and
            // a swing limit bounds both (a box, where avian's is a cone).
            let basis = Quat::from_rotation_arc(Vec3::Y, twist_axis.normalize_or(Vec3::Y));
            let mut b = rp::SphericalJointBuilder::new()
                .local_frame1(pose(joint.anchor1, basis))
                .local_frame2(pose(joint.anchor2, basis));
            if let Some(s) = swing_limit {
                b = b.limits(rp::JointAxis::AngX, [-s, s]).limits(rp::JointAxis::AngZ, [-s, s]);
            }
            if let Some(t) = twist_limit {
                b = b.limits(rp::JointAxis::AngY, [-t, t]);
            }
            b.into()
        }
        JointKind::Revolute { axis, limits } => {
            let axis = to_rapier(axis.normalize_or(Vec3::Y));
            let mut b = rp::RevoluteJointBuilder::new(axis).local_anchor1(a1).local_anchor2(a2);
            if let Some((min, max)) = limits {
                b = b.limits([min, max]);
            }
            b.into()
        }
        JointKind::Prismatic { axis, limits } => {
            let axis = to_rapier(axis.normalize_or(Vec3::X));
            let mut b = rp::PrismaticJointBuilder::new(axis).local_anchor1(a1).local_anchor2(a2);
            if let Some((min, max)) = limits {
                b = b.limits([min, max]);
            }
            b.into()
        }
        // A rope: rapier has a maximum distance but no minimum, so `min` goes
        // unenforced here.
        JointKind::Distance { max, .. } => {
            rp::RopeJointBuilder::new(max).local_anchor1(a1).local_anchor2(a2).into()
        }
    };
    built.set_contacts_enabled(joint.collide_connected);
    built
}

/// Build or rebuild `entity`'s joint. False when either body is not built
/// yet, so the caller can try again next frame.
fn build_joint(entity: Entity, joint: &PhysicsJoint, rapier: &mut Rapier3d) -> bool {
    rapier.remove_joint(entity);
    let (Some(b1), Some(b2)) = (rapier.bodies.get(&joint.body1).copied(), rapier.bodies.get(&joint.body2).copied())
    else {
        return false;
    };
    let handle = rapier.world.insert_impulse_joint(b1, b2, joint_for(joint));
    rapier.joints.insert(entity, handle);
    true
}

// ── Systems ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync(
    mut rapier: ResMut<Rapier3d>,
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
            Without<RuntimePhysics2d>,
            Or<(Added<RuntimePhysics>, Changed<PhysicsBodyData>)>,
        ),
    >,
    changed_colliders: Query<
        Entity,
        (With<RuntimePhysics>, Without<RuntimePhysics2d>, Changed<ResolvedCollider>),
    >,
    changed_joints: Query<Entity, Changed<PhysicsJoint>>,
    all_joints: Query<&PhysicsJoint>,
    dims: Query<Has<RuntimePhysics2d>>,
    moved: Query<(Entity, &Transform), (With<RuntimePhysics>, Without<RuntimePhysics2d>, Changed<Transform>)>,
    colliders: Query<&ResolvedCollider>,
    hierarchy: Query<(&Transform, Option<&ChildOf>)>,
    parents: Query<&ChildOf>,
    body_data: Query<&PhysicsBodyData>,
) {
    let rapier = &mut *rapier;

    // Gone first, so an entity removed and re-wired this frame comes back clean.
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
    // A body whose `PhysicsBodyData` went while the entity stays wired: the
    // body goes, and its own collider is rebuilt as a fixed one.
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
                // Mass rides on the body's own collider, so rebuild it.
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
                // A collider built before its body existed is standalone; now
                // it can attach.
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

    // A new body may be the one a waiting joint needs, or the return of one a
    // joint lost when its body was stripped.
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
        if dims.get(joint.body1) == Ok(true) {
            continue;
        }
        if !build_joint(entity, joint, rapier) {
            rapier.pending_joints.push(entity);
        }
    }

    // Anything moved by someone other than this backend: the editor's gizmo, a
    // script, the engine's collide-and-slide. A kinematic body is told where to
    // be next step; anything else is placed there now.
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

fn step(mut rapier: ResMut<Rapier3d>, settings: Res<PhysicsSettings>, time: Res<Time<Fixed>>) {
    if settings.paused {
        return;
    }
    let dt = time.delta_secs() * settings.time_scale;
    if dt <= 0.0 {
        return;
    }
    let world = &mut rapier.world;
    world.integration_parameters.dt = dt;
    // Avian's substeps and rapier's solver iterations both buy stiffer
    // contacts for time; one for one is the closest equivalent.
    world.integration_parameters.num_solver_iterations = settings.substeps.max(1) as usize;
    world.gravity = to_rapier(settings.gravity);
    world.step();
}

/// Dynamic bodies' poses back into their `Transform`s, relative to their
/// parent. Kinematic and static bodies are driven *by* their transforms, so
/// they are never written.
///
/// A parent that is itself a simulated body (a ragdoll bone's parent bone) is
/// read from rapier, not from its `GlobalTransform`: that was propagated
/// before this step moved it, and composing against it would leave every
/// child a step behind its parent, shaking the chain apart.
fn write_transforms(
    mut rapier: ResMut<Rapier3d>,
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
        let mut translation = to_bevy(body.translation());
        let mut rotation = rot_to_bevy(*body.rotation());
        if let Some(parent) = parent.map(|p| p.parent()) {
            let simulated = rapier
                .bodies
                .get(&parent)
                .and_then(|h| rapier.world.bodies.get(*h))
                .map(|b| (to_bevy(b.translation()), rot_to_bevy(*b.rotation())));
            let from_global = || {
                globals.get(parent).ok().map(|g| {
                    let (_, r, t) = g.to_scale_rotation_translation();
                    (t, r)
                })
            };
            if let Some((parent_translation, parent_rotation)) = simulated.or_else(from_global) {
                let inverse = parent_rotation.inverse();
                translation = inverse * (translation - parent_translation);
                rotation = inverse * rotation;
            }
        }
        if transform.translation != translation || transform.rotation != rotation {
            transform.translation = translation;
            transform.rotation = rotation;
        }
        rapier.written.insert(*entity, (translation, rotation));
    }
}

#[allow(clippy::type_complexity)]
fn write_state(
    rapier: Res<Rapier3d>,
    mut velocities: Query<(Entity, &mut BodyVelocity), Without<RuntimePhysics2d>>,
    mut contacts: Query<(Entity, &mut PhysicsContacts, Option<&GlobalTransform>), Without<RuntimePhysics2d>>,
) {
    for (entity, mut velocity) in &mut velocities {
        let v = rapier
            .bodies
            .get(&entity)
            .and_then(|h| rapier.world.bodies.get(*h))
            .map(|b| BodyVelocity { linear: to_bevy(b.linvel()), angular: to_bevy(b.angvel()) })
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
            // Manifold normals point from collider1 to collider2; ours points
            // at this body, which is the opposite way when it is collider1.
            let (point, normal) = match pair.find_deepest_contact() {
                Some((manifold, contact)) => {
                    let at = rapier
                        .world
                        .colliders
                        .get(pair.collider1)
                        .map(|c| to_bevy(c.position().transform_point(contact.local_p1)))
                        .unwrap_or(here);
                    let n = to_bevy(manifold.data.normal);
                    (at, if first { -n } else { n })
                }
                None => (here, Vec3::ZERO),
            };
            current.push(Contact { entity: other, point, normal, impulse: pair.total_impulse_magnitude() });
        }
        // A sensor never makes contact; it intersects.
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

/// This frame's commands for the 3D bodies among their targets. Forces from
/// last frame are dropped first, so one pushed every frame is a steady force.
fn apply_commands(
    mut rapier: ResMut<Rapier3d>,
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
        if dims.get(entity) != Ok(false) {
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
                body.add_torque(to_rapier(torque), true);
                rapier.forced.push(handle);
            }
            PhysicsCommand::AddImpulse { impulse, .. } => body.apply_impulse(to_rapier(impulse), true),
            PhysicsCommand::AddAngularImpulse { impulse, .. } => {
                body.apply_torque_impulse(to_rapier(impulse), true)
            }
            PhysicsCommand::SetLinearVelocity { velocity, .. } => body.set_linvel(to_rapier(velocity), true),
            PhysicsCommand::SetAngularVelocity { velocity, .. } => body.set_angvel(to_rapier(velocity), true),
            PhysicsCommand::Strip { .. } => {}
        }
    }
}

// ── Queries ──────────────────────────────────────────────────────────────

fn cast(rapier: &Rapier3d, world: &World, batch: &mut CastBatch) {
    let CastBatch { casts, hits } = batch;
    for (cast, hit) in casts.iter().zip(hits.iter_mut()) {
        if cast.dim != PhysicsDim::D3 {
            continue;
        }
        let Ok(dir) = Dir3::new(cast.direction) else { continue };
        let mask = cast.collides_with;
        let keep = |_: rp::ColliderHandle, c: &rp::Collider| {
            (cast.include_sensors || !c.is_sensor())
                && c.collision_groups().memberships.bits() & mask != 0
                && !cast.exclude.contains(&entity_from(c.user_data))
        };
        let filter = rp::QueryFilter::default().predicate(&keep);
        // Where the swept shape sits; a `Collider` cast adds the entity's own
        // offset and turn to the pose it is given.
        let mut at = pose(cast.origin, cast.rotation);
        let shape = match &cast.shape {
            CastShape::Ray => {
                let ray = rp::Ray::new(to_rapier(cast.origin), to_rapier(*dir));
                let solid = !cast.ignore_origin_penetration;
                if let Some((h, i)) = rapier.world.cast_ray_and_get_normal(&ray, cast.max_distance, solid, filter) {
                    if let Some(entity) = rapier.entity_of(h) {
                        *hit = Some(CastHit {
                            distance: i.time_of_impact,
                            point: cast.origin + *dir * i.time_of_impact,
                            normal: to_bevy(i.normal),
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
        let options = rapier3d::parry::query::ShapeCastOptions {
            max_time_of_impact: cast.max_distance,
            target_distance: 0.0,
            stop_at_penetration: !cast.ignore_origin_penetration,
            compute_impact_geometry_on_penetration: true,
        };
        if let Some((h, s)) = rapier.world.cast_shape(&at, to_rapier(*dir), &*shape, options, filter) {
            if let Some(entity) = rapier.entity_of(h) {
                *hit = Some(CastHit {
                    distance: s.time_of_impact,
                    point: to_bevy(s.witness1),
                    normal: to_bevy(s.normal1),
                    entity,
                });
            }
        }
    }
}

fn overlap(rapier: &Rapier3d, batch: &mut OverlapBatch) {
    let OverlapBatch { queries, hits } = batch;
    for (query, found) in queries.iter().zip(hits.iter_mut()) {
        if query.dim != PhysicsDim::D3 {
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
