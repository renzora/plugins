# Kuwahara

Edge-preserving smoothing: flat areas are flattened further while edges stay sharp. That is the painterly look without the brush clumps **Oil Painting** adds.

`radius` is the cost knob as well as the look knob, since the sample loop grows with its square.

**Add it:** `+ Add Entity → Post Process → Kuwahara`, or Add Component on an entity you already have.

**Settings:** `radius`

**Shader:** `src/kuwahara.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
