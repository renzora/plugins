# Pulse

A pulsing vignette: the edges of the frame darken and brighten on their own, for heartbeats, low health and anything that should feel like strain.

**It runs last.** Everything else in this set sorts at order 0 and this one at 1, because it darkens toward the edges: run it first and the other filters work on an already-vignetted picture, spreading the darkening into whatever they do. Only **Screen Transition** sorts after it.

For a still vignette, use the **Vignette** plugin.

**Add it:** `+ Add Entity → Post Process → Pulse`, or Add Component on an entity you already have.

**Settings:** `strength`, `speed`

**Shader:** `src/pulse.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
