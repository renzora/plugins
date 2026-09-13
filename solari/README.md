# Solari

Hardware ray-traced global illumination, wrapping Bevy's experimental `bevy_solari`: realtime raytraced direct and indirect lighting, fully dynamic, with nothing to bake.

**Requires a GPU with ray-query support, and the engine must have been told to ask for it at startup.** The wgpu ray-tracing features have to be enabled when the render device is created, long before a plugin loads, so Solari cannot switch them on by itself. If the capability is off the component still exists and still round-trips through scene files, it simply renders nothing.

**Add it:** Add Component → Solari Ray-Traced GI, in the Inspector. It is a lighting component, so it is not in the Add Entity list.

**Removing it:** delete the library from `plugins/`. Nothing in the engine references it.

**Note:** unlike the other GI paths, Solari reads no graphics quality tier, so lowering the tier will not switch it off.

**Scope:** Runtime (editor viewport and shipped game).
