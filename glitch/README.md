# Glitch

Digital glitch: blocks of the frame are displaced and their colour channels drift, the way a corrupt video stream tears. `time` advances on its own, so it keeps moving without anything to animate.

**Add it:** `+ Add Entity → Post Process → Glitch`, or Add Component on an entity you already have.

**Settings:** `intensity`, `block_size`, `color_drift`, `speed`

**Shader:** `src/glitch.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
