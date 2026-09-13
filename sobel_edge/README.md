# Sobel Edge

Sobel edge detection: the frame is replaced by its own gradient, so only edges survive.

For edges drawn over the picture rather than instead of it, use **Outline**; for glowing ones, **Edge Glow**.

**Add it:** `+ Add Entity → Post Process → Sobel Edge`, or Add Component on an entity you already have.

**Settings:** `intensity`, `threshold`

**Shader:** `src/sobel_edge.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
