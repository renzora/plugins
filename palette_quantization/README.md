# Palette Quantization

Reduces the frame to a fixed number of colours, with optional dithering to soften the banding that leaves. The retro-console counterpart to plain **Posterize**, which quantizes each channel independently.

**Add it:** `+ Add Entity → Post Process → Palette Quantization`, or Add Component on an entity you already have.

**Settings:** `num_colors`, `dithering`

**Shader:** `src/palette_quantization.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
