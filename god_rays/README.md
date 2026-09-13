# God Rays

Volumetric light shafts radiating from a point on screen: the frame is marched outward from the light position, accumulating brightness along the way.

The light position is authored in screen space (`light_pos_x` / `light_pos_y`, 0 to 1), so point it at wherever your sun lands in frame. `num_samples` is the cost knob.

**Add it:** `+ Add Entity → Post Process → God Rays`, or Add Component on an entity you already have.

**Settings:** `intensity`, `decay`, `density`, `num_samples`, `light_pos_x`, `light_pos_y`

**Shader:** `src/god_rays.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
