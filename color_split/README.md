# Color Split

Directional RGB split. Red and blue are pushed opposite ways along one axis and green is left where it is, which is why there are only two offsets to author. `angle` is in radians, so its maximum is a full turn.

**Add it:** `+ Add Entity → Post Process → Color Split`, or Add Component on an entity you already have.

**Settings:** `offset_r`, `offset_b`, `angle`

**Shader:** `src/color_split.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
