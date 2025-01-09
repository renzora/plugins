# Renzora Plugins

- Plugins are the backbone of Renzora and allow you to create anything you like from user interfaces, menus, scripts and more. To create a plugin you can follow the steps below:

Template Code
```
<div class="window window_bg text-white">
    <div class="window_title p-2">
        <span>Plugin Window</span>
    </div>
    <div class="container window_body text-center p-2">
        <p>Plugin content goes here</p>
        <button data-close class="white_button p-2 rounded mt-2" aria-label="Close">Okay</button>
    </div>
</div>
  
<style>

</style>
  
<script>
window[id] = {
    id: id,
    start: function() {
        console.log(`Plugin started: ${this.id}`);
    },

    unmount: function() {
        console.log(`Plugin unmounted: ${this.id}`);
    }
};
</script>
```

usage:
```
plugin.load({ id: 'template_plugin', url: 'template/index.html', drag: true, reload: true });
```

Once you have loaded the plugin, your `template_plugin` or any other id specified will be added to the global scope of the window. You can define variables within your plugin like follows:

```
window[id] = {
    id: id,
    custom_variable: 'hello renzora',
    start: function() {
        console.log(this.custom_variable);
    }
};
```

`this` refers to the local scope of the plugin, in this example `template_plugin`. If you wanted to access `custom_variable` from outside of the plugin you would use `template_plugin.custom_variable`

You can check if your plugin exists by using `if(utils.pluginExists('template_plugin'));` This is useful as a failsafe when using the plugins variables or functions in other areas of the code to protect against scripts breaking if you destroy your plugin.

You can load html and/or standalone javascript plugins. If you create a plugin that contains only javascript, it's good practise to put them into a .js file and load the plugin like below:
```
plugin.load({ id: 'template_plugin', url: 'template/index.js', reload: true });
```

You don't need to write any javascript for your plugins to work, you can create a plugin like this:
```
<div class="window window_bg text-white">
    <div class="window_title p-2">
        <span>Plugin Window</span>
    </div>
    <div class="container window_body text-center p-2">
        <p>Plugin content goes here</p>
        <button data-close class="white_button p-2 rounded mt-2" aria-label="Close">Okay</button>
    </div>
</div>
```

By default, if you set `drag: true` when loading the plugin, the plugin will have drag handles as long as you specify `window` as the parent class like this:
```<div class="window"></div>```

If you have a draggable plugin and you don't want to drag the content are you can add the class `window_body` to the area you wish to not drag.