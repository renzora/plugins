# Oil Painting

A simplified Kuwahara filter with luminance bucketing on top, which is what gives it visible brush clumps rather than the smooth flattening **Kuwahara** produces.

The sample loop is quadratic in `radius`, so that is the cost knob.

**Add it:** `+ Add Entity → Post Process → Oil Painting`, or Add Component on an entity you already have.

**Settings:** `radius`, `levels`

**Shader:** `src/oil_painting.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
