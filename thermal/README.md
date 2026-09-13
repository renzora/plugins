# Thermal Vision

A thermal camera look: luminance is mapped to a heat palette, from cold blues through to white-hot. `cold_threshold` sets where the palette starts.

It reads brightness, not temperature, so a bright cold surface still reads as hot.

**Add it:** `+ Add Entity → Post Process → Thermal Vision`, or Add Component on an entity you already have.

**Settings:** `intensity`, `contrast`, `cold_threshold`

**Shader:** `src/thermal.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
