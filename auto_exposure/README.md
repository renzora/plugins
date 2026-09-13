# Auto Exposure

A settings wrapper around Bevy's histogram auto-exposure. It builds a 64 bin luminance histogram each frame, throws away the darkest and brightest percentiles, and animates the camera's exposure so the remaining metered pixels average to middle grey. That percentile filtering is what keeps a dark or mostly empty scene from blowing out to white.

Because it always targets middle grey, a genuinely dark night scene will be lifted toward daylight unless you bias it. The compensation curve is there for that.

**Add it:** Add Component → Auto Exposure, in the Inspector. It is a camera component, so it is not in the Add Entity list.

**Note:** the Medium and High graphics quality tiers keep it; **Low switches it off**, and the Inspector header says so.

**Scope:** Runtime (editor viewport and shipped game).
