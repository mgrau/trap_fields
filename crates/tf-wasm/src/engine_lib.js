// JavaScript side of the engine's event hook: Rust calls tf_emit(ptr, len) with a JSON
// event; the worker sets Module.onEvent to forward it to the page.
addToLibrary({
  tf_emit: function (ptr, len) {
    const bytes = HEAPU8.slice(ptr, ptr + len);
    if (Module.onEvent) Module.onEvent(JSON.parse(new TextDecoder().decode(bytes)));
  },
});
