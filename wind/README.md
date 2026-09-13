# Wind

World-global wind, with two jobs.

**Evaluate.** It reads the World Environment's authored wind settings into one shared `WindState` resource every frame, gust envelope and smoothed sea state included. Grass, the cloud deck, the ocean and cloth all read that same resource, which is what stops them disagreeing about which way the wind is blowing.

**Sway.** Any mesh tagged **Wind Sway** gets a vertex-animated variant of its material, so trees, bushes and hand-modelled foliage move with it. Tag the model's **root**, not its child meshes.

**Add it:** wind itself is authored on the World Environment (the "Wind" section). For sway, Add Component → Wind Sway on a model root.

**Requires Bevy 0.19.1 or newer:** the sway prepass shader reads the material bind group, which 0.19.0 replaces with an empty layout on a depth-only prepass. On 0.19.0 the first swaying mesh to render trips a wgpu validation crash.

**Scripting:** declares two script functions for reading and setting the wind.

**Scope:** Runtime (editor viewport and shipped game).
