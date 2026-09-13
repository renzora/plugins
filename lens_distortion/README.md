# Lens Distortion

A thin wrapper around Bevy's built-in `LensDistortion`: barrel and pincushion warping with optional per-channel dispersion, rendered by Bevy's own post-process effect stack rather than by a shader shipped here.

Authored on an entity and routed onto the cameras, the same settings to sync to camera pattern the other built-in wrappers use.

**Add it:** `+ Add Entity → Effects → Lens Distortion`, or Add Component on an entity you already have.

**Scope:** Runtime (editor viewport and shipped game).
