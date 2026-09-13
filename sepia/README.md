# Sepia

Sepia tone mapping. The simplest effect in the set, and the one worth reading if you are writing your own: `#[post_process]` generates the derives, the uniform padding, the `enabled` flag, the render plugin and the inspector section, leaving only the two things the effect itself knows, its fields and its shader.

The tone weights are a tuned constant rather than three sliders, so they stay in the struct and out of the Inspector.

**Add it:** `+ Add Entity → Post Process → Sepia`, or Add Component on an entity you already have.

**Settings:** `intensity`

**Shader:** `src/sepia.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
