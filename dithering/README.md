# Dithering

Ordered (Bayer) dithering: colour is reduced to a small number of levels and the error is traded for a fixed crosshatch pattern, the way early 8-bit and 16-bit displays faked extra shades.

The threshold matrix is generated in the shader, so the effect ships no texture and the pattern stays pixel-exact at any render resolution.

**Add it:** `+ Add Entity → Post Process → Dithering`, or Add Component on an entity you already have.

**Settings:** `color_depth`, `intensity`

**Shader:** `src/dithering.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
