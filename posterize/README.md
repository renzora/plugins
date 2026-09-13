# Posterize

Quantizes each colour channel into `levels` steps, flattening smooth gradients into hard bands.

For a fixed total colour count with dithering, use **Palette Quantization** instead.

**Add it:** `+ Add Entity → Post Process → Posterize`, or Add Component on an entity you already have.

**Settings:** `levels`

**Shader:** `src/posterize.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
