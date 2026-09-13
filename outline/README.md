# Outline

Screen-space outlines drawn along detected edges.

`mix_mode` crossfades between drawing the outline over the picture and replacing the picture with it, so one effect covers both the comic-book look and a plain line render.

**Add it:** `+ Add Entity → Post Process → Outline`, or Add Component on an entity you already have.

**Settings:** `thickness`, `threshold`, `mix_mode`

**Shader:** `src/outline.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
