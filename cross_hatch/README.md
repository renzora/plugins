# Cross Hatch

Cross-hatch shading, as a pen-and-ink drawing shades with overlapping strokes: the darker the pixel, the more layers of hatching it gets.

`angle` is in radians and tops out at a quarter turn, because past that the hatching is the same set of lines again, mirrored.

**Add it:** `+ Add Entity → Post Process → Cross Hatch`, or Add Component on an entity you already have.

**Settings:** `density`, `thickness`, `angle`, `brightness`

**Shader:** `src/cross_hatch.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
