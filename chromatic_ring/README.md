# Chromatic Ring

Radial chromatic aberration: the colour split grows with distance from the centre of the frame rather than running one way across it, which is what a real lens does. `radius` is where it starts and `falloff` how quickly it climbs from there.

For a flat lateral split, use **Chromatic Aberration**.

**Add it:** `+ Add Entity → Post Process → Chromatic Ring`, or Add Component on an entity you already have.

**Settings:** `intensity`, `radius`, `falloff`

**Shader:** `src/chromatic_ring.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
