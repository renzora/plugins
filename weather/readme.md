# Weather Effects plugin for Renzora Engine

```
plugin.load({
    id: 'weather_plugin',
    url: 'effects/weather/index.js',
    drag: false,
    reload: true,
    after: function() {
        weather_plugin.snow.active = true;
    }
});
```