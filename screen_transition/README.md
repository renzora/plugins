# Screen Transition

Wipes and fades between scenes. Drive `progress` from a script or an animation: `0.0` is fully covered, `1.0` fully revealed.

It defaults to `1.0` on purpose. A transition that defaulted to `0.0` would blank the screen the moment it was added.

**It runs after everything, Pulse included.** A transition is the last thing between the finished frame and the player, so an effect running after it would filter the wipe itself and a fade to black would come out grey. It is also the one effect here that uses the framework's snapshot of the previous composited frame, which is what it wipes away from.

**Add it:** `+ Add Entity → Post Process → Screen Transition`, or Add Component on an entity you already have.

**Settings:** `progress`, `mode`, `direction`, `smoothness`

**Shader:** `src/screen_transition.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
