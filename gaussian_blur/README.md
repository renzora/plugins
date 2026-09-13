# Gaussian Blur

A plain Gaussian blur over the whole frame. `sigma` sets how soft it is, `kernel_size` how many samples pay for it, so raise them together: a large sigma through a small kernel is a blur with banding.

**Add it:** `+ Add Entity → Post Process → Gaussian Blur`, or Add Component on an entity you already have.

**Settings:** `sigma`, `kernel_size`

**Shader:** `src/gaussian_blur.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
