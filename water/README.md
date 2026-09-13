# Water

FFT ocean waves. The surface is a sum of **wave cascades**, each an independent simulation of a JONSWAP/TMA ocean spectrum over its own tile. Every frame the GPU propagates the spectrum in time and inverse-Fourier-transforms it into a displacement map and a normal/foam map; the material displaces its vertices by those maps and shades the result.

Foam comes from the Jacobian of the displacement, so it appears where the surface folds over itself, which is where waves actually break.

For a bounded pool or pond, use the **Pool Water** plugin instead.

**Add it:** `+ Add Entity → Rendering → Water Surface`.

**Settings:** wave resolution, mesh mode, mesh quality, cascade count and per-cascade setup, wind direction.

**Buoyancy:** add **Buoyant** to a rigid body and it floats on the real wave height. Settings: force, damping, submerge depth, wave push, drag.

**Wind:** the surface follows the world wind from the **Wind** plugin unless you override the direction.

Ported from [GodotOceanWaves](https://github.com/2Retr0/GodotOceanWaves) (MIT).

**Scope:** Runtime (editor viewport and shipped game).
