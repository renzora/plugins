# Kaleidoscope

Mirrors one wedge of the frame around the centre, so the picture repeats into a kaleidoscope. `segments` is how many wedges, `rotation` spins the whole pattern.

The centre is fixed at the middle of the screen, which is what a kaleidoscope wants. **Swirl** and **Radial Blur** are the effects in this set with a movable centre.

**Add it:** `+ Add Entity → Post Process → Kaleidoscope`, or Add Component on an entity you already have.

**Settings:** `segments`, `rotation`

**Shader:** `src/kaleidoscope.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
