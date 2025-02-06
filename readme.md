# Plugins
Plugins are the backbone of Renzora Engine and allow you to create anything you like from user interfaces, menus, scripts and more. To create a plugin you can follow the steps below:

## Template Code
``` js
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
template_plugin = {
    start: function() {
        console.log(`Plugin started: ${this.id}`);
    },

    unmount: function() {
        console.log(`Plugin unmounted: ${this.id}`);
    }
};
</script>
```

## usage
Please note if your plugin is javascript, you don't need to specify an `ext`
``` js
// javascript
plugin.load({ id: 'template_plugin' });
```

``` js
// html
plugin.load({ id: 'template_plugin', ext: 'html' });
```

``` js
// Nunjucks
plugin.load({ id: 'template_plugin', ext: 'njk' });
```

``` js
// some_directory/template_plugin/index.js
plugin.load({ id: 'template_plugin', path: 'some_directory' });
```

Once you have loaded the plugin, your `template_plugin` or any other id specified will be added to the global scope of the window. You can define variables within your plugin like follows:

``` js
template_plugin = {
    custom_variable: 'hello renzora',
    start: function() {
        console.log(this.custom_variable);
    }
};
```

`this` refers to the local scope of the plugin, in this example `template_plugin`. If you wanted to access `custom_variable` from outside of the plugin you would use `plugin.template_plugin.custom_variable`

## preload
You can preload multiple plugins at the same time using `plugin.preload()`
``` js
plugin.preload([
    { id: "time", path: "core" },
    { id: "notif", path: "core", ext: "html", after: function () {
        notif.show("notification", "hello world", "success");
    }},
    { id: "auth", ext: "njk" },
    { id: 'lighting' },
    { id: 'collision' },
    { id: 'pathfinding' },
    { id: 'debug', path: 'core', ext: 'html' },
    { id: 'ui', path: 'core' },
    { id: 'gamepad' },
    { id: 'snow', reload: true }
]);

```

## plugin.Exists
You can check if your plugin exists by using `if(plugin.exists('template_plugin'))` anywhere in the engines code.

You can also use `plugin.template_plugin.function()`. Adding `plugin.` before your plugin object is useful as a failsafe when using the plugins variables or functions in other areas of the code to protect against scripts breaking if you destroy your plugin or it's not loaded.

You don't need to write any javascript for your plugins to work, you can create a plugin like this:
``` html
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

## API Reference

The global `plugin` object provides a comprehensive API to dynamically load, manage, and interact with plugins. Below is a reference guide for all available methods and properties.

---

### Global Proxy Object: `plugin`

The `plugin` object is a proxy that encapsulates the entire plugin system. It routes method calls to the underlying implementation and also allows access to plugin objects defined in the global scope. When accessing properties not defined on the proxy, it returns a plugin object (or a no-operation function if not found).

---

### Methods

### `plugin.init(selector, options)`
Initializes a plugin window.

- **Parameters:**
  - `selector` *(string)*: A CSS selector to locate the plugin element.
  - `options` *(object, optional)*: Configuration options. May include:
    - `drag` *(boolean)*: Enable or disable draggable functionality.
    - `start`, `drag`, `stop` *(functions)*: Callbacks for drag events.

- **Usage:**  
  Initializes the element found via the selector, applies draggable behavior (if enabled), sets the initial z-index, and adds the element to the plugin stack.

---

### `plugin.front(elementOrId)`
Brings the specified plugin window to the front.

- **Parameters:**
  - `elementOrId` *(HTMLElement | string)*: Either the plugin element or its identifier (`data-window` value).

- **Usage:**  
  Reorders the plugin stack so that the specified plugin becomes the active window, updating z-indexes accordingly.

---

### `plugin.initDraggable(element, options)`
Enables draggable functionality on a plugin window element.

- **Parameters:**
  - `element` *(HTMLElement)*: The plugin element to make draggable.
  - `options` *(object, optional)*: May include drag event callbacks (`start`, `drag`, `stop`).

- **Usage:**  
  Attaches mouse and touch event listeners to facilitate dragging, and calls the provided callbacks during the drag lifecycle.

---

### `plugin.initCloseButton(element)`
Initializes the close button functionality for a plugin window.

- **Parameters:**
  - `element` *(HTMLElement)*: The plugin window element.

- **Usage:**  
  Searches for a child element with the `data-close` attribute and attaches a click listener to close the window.

---

### `plugin.minimize(id)`
Minimizes (hides) the plugin window with the given ID.

- **Parameters:**
  - `id` *(string)*: The identifier of the plugin window.

- **Usage:**  
  Sets the display style of the window to `none` and brings the next available plugin to the front.

---

### `plugin.load(id, options)`
Dynamically loads a plugin. Supports JavaScript, HTML, and Nunjucks templates.

- **Parameters:**
  - `id` *(string)*: Unique identifier for the plugin.
  - `options` *(object, optional)*: Configuration options which include:
    - `path` *(string)*: Directory path to the plugin.
    - `ext` *(string)*: File extension (`js`, `html`, or `njk`).
    - `reload` *(boolean)*: Force reload if already loaded.
    - `hidden` *(boolean)*: Hide the plugin after loading.
    - `before` *(function)*: Callback before loading begins.
    - `beforeStart` *(function)*: Callback before the plugin’s `start` method is invoked.
    - `after` *(function)*: Callback after the plugin has loaded.
    - `onError` *(function)*: Callback for handling errors.
    - `drag` *(boolean)*: Enable draggable behavior.

- **Returns:**  
  A Promise that resolves once the plugin is successfully loaded.

- **Usage:**  
  Loads the plugin from the appropriate URL (based on the `ext` and `path`), injects its content or script into the document, and initializes it.

---

### `plugin.close(id, fromEscKey)`
Closes the plugin window with the specified ID.

- **Parameters:**
  - `id` *(string)*: The unique identifier of the plugin.
  - `fromEscKey` *(boolean, optional)*: If `true`, the close request originated from the Escape key, respecting the `data-close` attribute.

- **Usage:**  
  Removes the plugin element from the DOM, unmounts it (if applicable), and updates the plugin stack.

---

### `plugin.unmount(id)`
Unmounts a plugin and cleans up its resources.

- **Parameters:**
  - `id` *(string)*: The unique identifier of the plugin.

- **Usage:**  
  Calls the plugin’s `unmount` method (if it exists), clears event listeners and other properties, and deletes the plugin from the global scope.

---

### `plugin.preload(pluginList)`
Preloads multiple plugins simultaneously.

- **Parameters:**
  - `pluginList` *(Array)*: An array of plugin configuration objects. Each object can include properties such as `id`, `path`, `ext`, etc.

- **Usage:**  
  Queues up plugins to be loaded one after the other.

---

### `plugin.loadNextPreload()`
Loads the next plugin in the preload queue.

- **Usage:**  
  Internal method that recursively loads all plugins queued by `plugin.preload()`.

---

### `plugin.hook(hookName)`
Executes a hook function on all loaded plugins that implement it.

- **Parameters:**
  - `hookName` *(string)*: The name of the hook to trigger.

- **Usage:**  
  Iterates over each loaded plugin and calls the specified hook if the plugin defines it.

---

### `plugin.topZIndex()`
Calculates and returns the highest z-index value present in the document.

- **Returns:**  
  *(number)*: The maximum z-index found among all elements.

- **Usage:**  
  Helps determine the starting z-index for plugin windows.

---

### `plugin.getActivePlugin()`
Returns the identifier of the currently active plugin window.

- **Returns:**  
  *(string | null)*: The `data-window` value of the active plugin or `null` if none are active.

---

### `plugin.show(id)`
Displays the plugin window with the specified ID and brings it to the front.

- **Parameters:**
  - `id` *(string)*: The unique identifier of the plugin.

- **Usage:**  
  Sets the display style to `block` and updates the plugin stack.

---

### `plugin.showAll()`
Makes all plugin windows visible and brings the last plugin to the front.

- **Usage:**  
  Iterates through all elements with a `data-window` attribute, setting their display to `block`.

---

### `plugin.hideAll()`
Hides all plugin windows and resets the active plugin.

- **Usage:**  
  Iterates through all plugin windows, setting their display to `none`.

---

### `plugin.closeAll()`
Closes and unmounts all plugin windows.

- **Usage:**  
  Removes all elements with a `data-window` attribute from the DOM and cleans up their associated resources.

---

### `plugin.closest(element)`
Finds the closest ancestor element that represents a plugin window.

- **Parameters:**
  - `element` *(HTMLElement)*: The starting point for the search.

- **Returns:**  
  *(string | null)*: The `data-window` attribute of the closest plugin element, or `null` if not found.

---

### `plugin.isVisible(id)`
Checks whether the plugin window with the given ID is currently visible.

- **Parameters:**
  - `id` *(string)*: The unique identifier of the plugin.

- **Returns:**  
  *(boolean)*: `true` if the plugin is visible; otherwise, `false`.

---

### `plugin.exists(...objNames)`
Verifies that the specified plugin objects exist in the global scope.

- **Parameters:**
  - `...objNames` *(string)*: One or more plugin object names to check.

- **Returns:**  
  *(boolean)*: `true` if all specified plugins exist; otherwise, `false`.

---

### Proxy Behavior

The `plugin` object is implemented as a proxy. This means:
- Accessing a method like `plugin.template_plugin.someFunction()` will work if `template_plugin` is loaded and defines `someFunction()`.
- If the plugin or the requested function does not exist, a no-operation function is returned to prevent runtime errors.

---

This API reference serves as a guide for interacting with the plugin system. Use these methods to load, display, manipulate, and manage plugins dynamically within your application.
