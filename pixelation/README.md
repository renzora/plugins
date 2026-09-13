# Pixelation

Snaps the frame to a coarse square grid, for a low-resolution look. `pixel_size` is in pixels, so the blocks stay the same size whatever the render resolution.

For tiles with grout lines and rounded corners, use **Mosaic**; for a hex grid, **Hex Pixelate**.

**Add it:** `+ Add Entity → Post Process → Pixelation`, or Add Component on an entity you already have.

**Settings:** `pixel_size`

**Shader:** `src/pixelation.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
