# Grayscale

Drains colour from the frame. `intensity` crossfades, so it doubles as a partial desaturation rather than only an on/off black and white.

The luminance weights are the Rec. 709 constants and are deliberately not exposed: they are values the shader reads every pixel, not three sliders worth having.

**Add it:** `+ Add Entity → Post Process → Grayscale`, or Add Component on an entity you already have.

**Settings:** `intensity`

**Shader:** `src/grayscale.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
