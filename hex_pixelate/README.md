# Hex Pixelate

Pixelation on a hexagonal grid rather than a square one.

The grid is computed in axial coordinates in the shader, so `hex_size` is a pixel width rather than a cell count: the pattern stays the same size whatever the render resolution.

**Add it:** `+ Add Entity → Post Process → Hex Pixelate`, or Add Component on an entity you already have.

**Settings:** `hex_size`

**Shader:** `src/hex_pixelate.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
