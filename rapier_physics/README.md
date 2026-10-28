# Rapier Physics

[Rapier](https://rapier.rs) as a physics backend for Renzora, for both worlds:
`rapier3d` for 3D scenes and `rapier2d` for sprite scenes.

Install it and nothing changes until you pick it: **Project Settings > Physics >
3D Backend > rapier3d** and/or **2D Backend > rapier2d**. The editor moves the
scene onto Rapier straight away, and the game uses it from its next start. Pick
**Default** to go back to the bundled avian. Each world is chosen on its own,
so a project can run 3D on Rapier and 2D on avian, or the other way round.

Everything you author stays the same: Physics Body and Collision Shape
components, joints, collision layers, `apply_force`, `set_velocity`,
`move_controller`, collision events, ragdolls, parkour, navmesh obstacles,
buoyancy and particle collisions all run through the engine's physics API, so a
scene behaves the same on either engine.

## What it covers

- Dynamic, static and kinematic bodies, with mass, gravity scale, damping, axis
  locks and continuous collision detection.
- Box, sphere, capsule, cylinder and mesh colliders (rectangle, circle and
  capsule in 2D), with offset and rotation, sensors, friction, restitution and
  collision layers, scaled by the entity's transform.
- Colliders on child entities attach to the nearest body above them.
- Fixed, ball (3D), hinge, slider and rope joints.
- Forces, torques, impulses and velocities from scripts and engine code.
- Contacts with their point, normal and impulse, for collision events.
- Ray casts, shape casts and overlap queries.

## Differences from avian

These come from how the two engines work, not from anything missing here:

- A joint's softness (`compliance`) is not used. Rapier softens joints with
  springs rather than compliance, and no single conversion is right for every
  mass, so Rapier joints are rigid.
- A distance joint keeps only its maximum length (a rope); its minimum is not
  enforced.
- A ball joint's swing limit is a box (the same limit about two axes) rather
  than avian's cone.
