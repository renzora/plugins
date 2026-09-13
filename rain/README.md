# Rain

Rain on the lens: droplets and runnels over the finished frame, as if the camera itself is wet. It is a screen effect, not weather in the world.

**Add it:** `+ Add Entity → Post Process → Rain`, or Add Component on an entity you already have.

**Settings:** `intensity`, `speed`, `drop_size`, `speed_variation`, `trail`, `fog`

The glass fogs, and the water running down it wipes the fog away — that contrast
is what reads as a wet windscreen rather than marks on the picture. `fog` sets how
much it hazes between the water; 0 leaves the glass clear.

`drop_size` is a size, so turning it up makes fewer, larger beads.
`speed_variation` spreads the fall rate between columns — 0 runs them all at
`speed`. `trail` is the length of the runnel each bead drags, as a multiple of its
own size; 0 leaves bare beads.

Refraction comes from the slope of the water rather than being drawn per drop, so
beads and runnels bend light by the same rule. That costs three evaluations of the
drop field per pixel plus nine taps for the fog blur — the screen target has no
mip chain to blur against.

**Shader:** `src/rain.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
