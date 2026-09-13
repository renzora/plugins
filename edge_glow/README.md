# Edge Glow

Edge detection with the edges lit up: outlines glow rather than being drawn as flat lines. Suits scanner overlays, hologram looks and anything meant to read as a synthetic view.

**Add it:** `+ Add Entity → Post Process → Edge Glow`, or Add Component on an entity you already have.

**Settings:** `threshold`, `glow_intensity`

**Shader:** `src/edge_glow.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
