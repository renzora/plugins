# 3D Text

Renders a string as glyph geometry in world space. The engine otherwise has only the stroke-font debug gizmo, so this is what you want for signage, labels and titles that live in the scene.

Two modes:

- **flat**: the font is rasterized through Bevy's atlas, packed into a signed distance field, and emitted as one textured quad per glyph. Cheap and crisp at any distance, but a flat card.
- **mesh**: glyph outlines are read from the font file and triangulated (holes and all), optionally extruded. Real 3D letters that catch light.

**Add it:** `+ Add Entity → Basic → 3D Text`, or Add Component → 3D Text.

**Settings:** font, mode, colour.

**Scope:** Runtime (editor viewport and shipped game).
