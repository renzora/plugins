# Weather Effects plugin for Renzora Engine

```
plugin.load({
    id: 'weather_plugin',
    url: 'effects/weather/index.js',
    reload: true,
    after: function() {
        weather_plugin.snow.active = true;
    }
});
```