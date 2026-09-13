# ASCII

ASCII art post-process effect. The character shapes are generated in the shader from luminance rather than sampled from a font atlas, so there is no texture to ship and the cell size can be any number rather than a multiple of a glyph.

**Add it:** `+ Add Entity → Post Process → ASCII`, or Add Component on an entity you already have. The effect is routed onto the cameras for you.

**Settings:** `char_size`, `color_mix`, `contrast`

**Shader:** `src/ascii.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
