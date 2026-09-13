# Toon

Cel shading: luminance is quantized into bands and an edge is drawn over the result.

It is **Posterize** and **Outline** in one pass rather than two, which is the point: doing it together keeps the edge aligned to the bands it is outlining.

**Add it:** `+ Add Entity → Post Process → Toon`, or Add Component on an entity you already have.

**Settings:** `levels`, `edge_threshold`, `edge_thickness`, `saturation_boost`

**Shader:** `src/toon.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
