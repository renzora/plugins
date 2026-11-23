# Dynamic Weather

Weather that changes. Pick **Clear, Partly Cloudy, Overcast, Rain, Storm, Foggy or Snow** and the world blends into it over a transition time:

- **The clouds:** how much of the sky they cover, how thick and low they sit, and their colour.
- **The light:** the sun dims and greys under cloud, and so does the sky's ambient light, so the city darkens with the sky.
- **The air:** haze and fog close in.
- **The wind:** it picks up and gusts.

Rain falls around the camera, a storm brings lightning bolts forking down out of the clouds, and **Snow** brings a bright, soft winter sky with flakes drifting down. Turn on **Cycle** and the weather changes by itself, building and clearing in stages: a clear sky clouds over before it rains or snows, and a storm eases into rain before it clears.

Every weather is built from the look your scene already has. A storm over a golden-hour city is a golden-hour storm, and **Clear** is your scene exactly as you made it. Switch Weather off or remove it and that look comes back.

**Add it:** select the World Environment, then **Add Component → Rendering → Weather**.

**Settings.** The Inspector shows only what applies to the weather you are in: rain settings in rain and storms, lightning in a storm, snow while it snows and while snow still lies.

| Setting | What it does |
|---|---|
| Weather | The weather to be in, or to blend toward. |
| Transition (s) | How long a change takes. Real weather turns over in minutes. |
| Intensity | How strongly every weather darkens, fogs and blows. 1 is as tuned; 0 is your scene unchanged; up to 2. |
| Fog | How much haze the weather adds, 0 to 3. 0 keeps your scene's own haze whatever the weather. |
| **Rain** | |
| Show Rain | Draw falling rain around the camera. |
| Rain Amount | How hard it rains, 0 to 2. More draws more rain and soaks the streets faster. |
| **Lightning** | |
| Lightning | Flashes in the clouds during a storm. |
| Bolts | Draw the bolt itself, branching down from the cloud to the ground, as well as the flash. |
| Frequency | How often it strikes, 0.05 to 5 times the storm's own rate. |
| Brightness | How brightly a strike lights the clouds, the city and the bolt, 0 to 3. |
| **Snow** | |
| Show Snow | Draw falling snow around the camera. |
| Snow Amount | How hard it snows, 0 to 2. More draws more flakes and covers the ground faster. |
| Time to Cover (s) | Seconds of snowfall to cover the ground completely. Default 150. |
| Melt Time (s) | Seconds for a full cover to melt once the snow stops. Default 120. Rain melts it four times as fast, so a storm washes a snowy street clear in about 30 seconds. |
| **Cycle** | Change weather by itself, holding each one between **Hold Min** and **Hold Max** seconds. |
| Allow Snow | Let the cycle reach Snow. Off by default, so a warm scene never snows by itself; picking Snow yourself always works. |

**Editing your scene's look:** while Weather is on, it owns the clouds, the sun's strength and colour, the Environment Map's intensity, the distance fog and the wind. To change those, switch Weather off, edit, and switch it back on. It takes a fresh snapshot of the new look.

**Scripting:**

```lua
set_weather("storm", 120)      -- blend into a storm over two minutes
set_weather("snow", 300)       -- let it start snowing over five minutes
set_weather("clear", -1)       -- use the Weather's own transition time
set_weather_cycle(true)        -- let it change by itself
local wet = get("WeatherState.wetness")
```

`WeatherState` holds `cloudiness`, `rain`, `wetness`, `snow`, `snow_cover` and `lightning`, each 0 to 1. Wetness lags the rain: streets soak through over a minute or two and take longer to dry, so they still shine after a shower passes. Snow cover builds over two or three minutes of snowfall and melts over about fifteen once it stops. Weather publishes wetness for your materials to use; it does not wet surfaces by itself.

**Lying snow:** as snow cover builds, every surface that faces the sky turns white: roofs, ledges, awnings, roads and terrain, flattest and most exposed first, in drifts, until a full cover reaches gentler slopes. It is laid into the renderer's G-buffer, so it works on every material without changing any of them, and it is lit, shadowed and ray-traced like the surface it is on. It needs the deferred renderer (the default); a forward-rendered camera, glass and water stay bare.

**Needs:** the engine's Clouds (for the sky) and Particles (for the rain and snow). Without them, Weather still drives the sun, haze and wind.

**Scope:** Runtime (editor viewport and shipped game).
