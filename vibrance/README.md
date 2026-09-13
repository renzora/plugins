# Vibrance

Saturation weighted by how unsaturated a pixel already is, so it lifts muted colour without pushing already-vivid pixels further.

`intensity` goes negative, which desaturates the same way round.

**Add it:** `+ Add Entity → Post Process → Vibrance`, or Add Component on an entity you already have.

**Settings:** `intensity`

**Shader:** `src/vibrance.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
