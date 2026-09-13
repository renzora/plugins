# CRT

Screen curvature, scanlines, a colour fringe and a vignette in one pass.

Each of those exists separately in this set, but a CRT wants them sharing a curved UV: the scanlines have to bend with the glass, and stacking four separate passes would leave them flat over a curved picture.

**Add it:** `+ Add Entity → Post Process → CRT`, or Add Component on an entity you already have.

**Settings:** `scanline_intensity`, `curvature`, `chromatic_amount`, `vignette_amount`

**Shader:** `src/crt.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
