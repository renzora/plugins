# Changelog

## 1.1.1

- fix: builds on the engine's physics backend split (needs the r1-alpha8 nightly from 2026-10-24 or later). `spawn_entity`, `make_solid` and `duplicate_entity` go through `renzora::physics` (`CollisionShapeData` / `PhysicsBodyData`) instead of the contract crate's avian re-export, which is gone; mesh colliders are now built by the engine from the entity's `Mesh3d`, and the copy made by `duplicate_entity` finds the backend's collider↔body relationship components through the type registry rather than naming avian's.

## 1.1.0

- feat: `key_pose` keys bones' current pose at the timeline's playhead, as K over the viewport does. Keying into bone tracks needs the r1-alpha8 nightly from 2026-10-23 or later; an older engine keys a property track on the bone instead.

## 1.0.1

- Builds on Bevy 0.20 (needs the r1-alpha8 nightly from 2026-10-19 or later).
- feat: a component's enum field can be set by its variant name.

## 1.0.0

- Initial release.
