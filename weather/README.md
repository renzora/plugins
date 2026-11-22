# Dynamic Weather

Weather that changes. Pick **Clear, Partly Cloudy, Overcast, Rain, Storm or Foggy** and the world blends into it over a transition time:

- **The clouds:** how much of the sky they cover, how thick and low they sit, and their colour.
- **The light:** the sun dims and greys under cloud, and so does the sky's ambient light, so the city darkens with the sky.
- **The air:** haze and fog close in.
- **The wind:** it picks up and gusts.

A storm brings rain falling around the camera and lightning flickering inside the clouds. Turn on **Cycle** and the weather changes by itself, building and clearing in stages: a clear sky clouds over before it rains, and a storm eases into rain before it clears.

Every weather is built from the look your scene already has. A storm over a golden-hour city is a golden-hour storm, and **Clear** is your scene exactly as you made it. Switch Weather off or remove it and that look comes back.

**Add it:** select the World Environment, then **Add Component → Rendering → Weather**.

**Settings:**

| Setting | What it does |
|---|---|
| Weather | The weather to be in, or to blend toward. |
| Transition (s) | How long a change takes. Real weather turns over in minutes. |
| Intensity | How strongly every weather darkens, fogs and blows. 1 is as tuned; 0 is your scene unchanged; up to 2. |
| Cycle | Change weather by itself, holding each one between **Hold Min** and **Hold Max** seconds. |
| Rain | Draw falling rain around the camera when it rains. |
| Lightning | Lightning flashes in a storm. |

**Editing your scene's look:** while Weather is on, it owns the clouds, the sun's strength and colour, the Environment Map's intensity, the distance fog and the wind. To change those, switch Weather off, edit, and switch it back on. It takes a fresh snapshot of the new look.

**Scripting:**

```lua
set_weather("storm", 120)      -- blend into a storm over two minutes
set_weather("clear", -1)       -- use the Weather's own transition time
set_weather_cycle(true)        -- let it change by itself
local wet = get("WeatherState.wetness")
```

`WeatherState` holds `cloudiness`, `rain`, `wetness` and `lightning`, each 0 to 1. Wetness lags the rain: streets soak through over a minute or two and take longer to dry, so they still shine after a shower passes.

**Needs:** the engine's Clouds (for the sky) and Particles (for the rain). Without them, Weather still drives the sun, haze and wind.

**Scope:** Runtime (editor viewport and shipped game).
