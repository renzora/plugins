# Vignette

A thin wrapper around Bevy's built-in `Vignette`: darkening toward the edges of the frame, rendered by Bevy's own post-process effect stack rather than by a shader shipped here.

Authored on an entity and routed onto the cameras, the same settings to sync to camera pattern the other built-in wrappers use.

**Add it:** `+ Add Entity → Effects → Vignette`, or Add Component on an entity you already have.

**See also:** the **Pulse** plugin, which is an animated vignette with its own shader.

**Scope:** Runtime (editor viewport and shipped game).
