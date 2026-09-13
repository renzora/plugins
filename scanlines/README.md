# Scanlines

CRT scanlines: horizontal dark lines across the frame, optionally drifting.

For the full CRT treatment (curvature, colour fringe and vignette sharing one curved UV), use the **CRT** plugin instead of stacking this with the others.

**Add it:** `+ Add Entity → Post Process → Scanlines`, or Add Component on an entity you already have.

**Settings:** `intensity`, `count`, `speed`

**Shader:** `src/scanlines.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
