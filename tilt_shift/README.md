# Tilt Shift

Blurs the top and bottom of the frame, leaving a sharp band in the middle, which reads as a miniature.

A screen-space fake rather than a real depth of field: the focus band is a horizontal strip at `focus_position`, so it costs nothing to sample the depth buffer and works on a 2D scene that has none. For real depth of field, use the engine's own **Depth of Field**.

**Add it:** `+ Add Entity → Post Process → Tilt Shift`, or Add Component on an entity you already have.

**Settings:** `blur_amount`, `focus_position`, `focus_width`, `focus_falloff`

**Shader:** `src/tilt_shift.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
