# Distortion

Animated wave distortion: the picture is displaced by a scrolling noise field. Good for dream sequences, damage effects and anything meant to look unstable.

`time` advances on its own, so there is nothing to animate by hand.

**Add it:** `+ Add Entity → Post Process → Distortion`, or Add Component on an entity you already have.

**Settings:** `intensity`, `speed`, `scale`

**Shader:** `src/distortion.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
