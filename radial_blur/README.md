# Radial Blur

Streaks the frame outward from a point, for speed and impact shots.

Unlike **Swirl** and **Kaleidoscope**, the centre is authored: a radial blur is usually aimed at something, so the point it streaks away from is the whole control.

**Add it:** `+ Add Entity → Post Process → Radial Blur`, or Add Component on an entity you already have.

**Settings:** `intensity`, `center_x`, `center_y`, `samples`

**Shader:** `src/radial_blur.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
