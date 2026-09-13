# Threshold

Cuts the frame to black and white at a brightness threshold.

`smoothness` is what keeps the cut from aliasing: at zero the split is a hard step and every edge in the picture stairsteps, so the default eases it over a narrow band either side.

**Add it:** `+ Add Entity → Post Process → Threshold`, or Add Component on an entity you already have.

**Settings:** `threshold`, `smoothness`

**Shader:** `src/threshold.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
