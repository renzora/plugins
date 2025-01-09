# Renzora Plugins

## Plugins are the backbone of Renzora and allow you to create anything you like from user interfaces, menus, scripts and more. To create a plugin you can follow the steps below:

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