# Wave

Sine wave distortion: the frame ripples along a travelling sine. Simpler and more regular than **Distortion**, which displaces by noise.

`time` advances on its own.

**Add it:** `+ Add Entity → Post Process → Wave`, or Add Component on an entity you already have.

**Settings:** `amplitude`, `frequency`, `speed`

**Shader:** `src/wave.wgsl` · **Scope:** Runtime (editor viewport and shipped game)
