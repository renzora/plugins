# Fog Overlay

A cheap height-fade over the finished picture. It is a screen-space overlay, not a depth-aware fog: it knows how far up the frame a pixel is, not how far away it is.

For fog that respects distance, use the engine's own **Distance Fog** on the World Environment instead.

**Add it:** `+ Add Entity → Post Process → Fog Overlay`, or Add Component on an entity you already have.

**Settings:** `density`, `height`

**Shader:** `src/fog_overlay.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
