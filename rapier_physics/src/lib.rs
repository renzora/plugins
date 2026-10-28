//! Rapier as a physics backend, for both worlds: `rapier3d` and `rapier2d`.
//!
//! Installs beside the bundled avian and does nothing until a project picks it:
//! Project Settings > Physics > 3D or 2D backend (or `[physics] backend_3d =
//! "rapier3d"` / `backend_2d = "rapier2d"` in `project.toml`). The engine then
//! detaches avian from that world, re-wires every entity in it, and this plugin
//! builds them in its own.
//!
//! Avian keeps its state as Bevy components; Rapier keeps a world of its own.
//! So this backend is the other half of the contract's design: it mirrors the
//! engine's components (`RuntimePhysics`, `PhysicsBodyData`, `ResolvedCollider`,
//! `PhysicsJoint`) into rapier handles, keyed by entity, and writes motion back.
//! Nothing of rapier's ever lands on an entity, which is also why scene saves
//! need no filtering for it.
//!
//! What differs from avian, by design of the engines rather than of this
//! plugin: a joint's `compliance` is not carried over (rapier softens joints
//! with springs, not compliance), a distance joint enforces its maximum only,
//! and a spherical joint's swing limit is a box rather than a cone.

mod world2d;
mod world3d;

use bevy::prelude::*;
use renzora::physics::{PhysicsCommands, PhysicsSettings};

pub use world2d::{Rapier2d, Rapier2dBackend};
pub use world3d::{Rapier3d, Rapier3dBackend};

pub struct RapierPhysicsPlugin;

impl Plugin for RapierPhysicsPlugin {
    fn build(&self, app: &mut App) {
        info!("[plugin] RapierPhysicsPlugin");
        app.init_resource::<PhysicsSettings>().init_resource::<PhysicsCommands>();
        world3d::build(app);
        world2d::build(app);
    }
}

renzora::plugin!(RapierPhysicsPlugin, Runtime);

// ── Shared by both worlds ────────────────────────────────────────────────

/// An entity id in a rapier `user_data` slot, so a query hit names the entity.
fn entity_bits(entity: Entity) -> u128 {
    entity.to_bits() as u128
}

fn entity_from(bits: u128) -> Entity {
    Entity::from_bits(bits as u64)
}

/// An entity's world transform, composed from its own and its ancestors' local
/// transforms. Not `GlobalTransform`: that is only propagated in `PostUpdate`,
/// so for a scene spawned this frame it would still read identity.
fn world_transform(entity: Entity, hierarchy: &Query<(&Transform, Option<&ChildOf>)>) -> Transform {
    let Ok((local, parent)) = hierarchy.get(entity) else {
        return Transform::IDENTITY;
    };
    let mut world = *local;
    let mut next = parent.map(|p| p.parent());
    while let Some(e) = next {
        let Ok((t, parent)) = hierarchy.get(e) else { break };
        world = t.mul_transform(world);
        next = parent.map(|p| p.parent());
    }
    world
}
