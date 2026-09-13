# Pillarbox

Black bars left and right: the vertical counterpart to **Letterbox**, for a narrow picture on a wide display.

`aspect_ratio` at zero means "use `bar_width` directly". Give it a real ratio and the bars size themselves to crop the view to it.

**Add it:** `+ Add Entity → Post Process → Pillarbox`, or Add Component on an entity you already have.

**Settings:** `bar_width`, `softness`, `aspect_ratio`

**Shader:** `src/pillowbox.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
