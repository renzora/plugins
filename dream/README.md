# Dream

A dreamy soft glow: a thresholded blur mixed back over the picture.

Unlike bloom it keeps the blurred copy visible in the midtones rather than only where the image is bright, which is what gives it a hazy look instead of a highlight sheen.

**Add it:** `+ Add Entity → Post Process → Dream`, or Add Component on an entity you already have.

**Settings:** `intensity`, `blur_radius`, `threshold`

**Shader:** `src/dream.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
