# Chromatic Aberration

Splits the colour channels across the frame, the way a cheap lens does. The direction is a fixed horizontal split rather than a setting: a directional aberration reads as a camera fault, and the one people reach for is lateral.

For the radial version, where the split grows with distance from the centre, use **Chromatic Ring**.

**Add it:** `+ Add Entity → Post Process → Chromatic Aberration`, or Add Component on an entity you already have.

**Settings:** `intensity`, `samples`

**Shader:** `src/chromatic_aberration.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
