# API

The engine runs inside your app, on the thread that creates it. There is no separate process
and no IPC. Widgets live as long as the app.

## Rust

```toml
[dependencies]
widget-engine = { path = "crates/engine" }
```

```rust
use widget_engine::Engine;

let mut engine = Engine::new();                       // per-monitor DPI aware, COM initialized
engine.load("weather", "widgets/weather", 100, 100)?; // id, package dir, default x/y (pixels)
engine.set("weather", "city", "Tokyo");               // {app.city} in layout.xml
engine.on_action(|widget, action| { /* action="..." */ });
engine.on_paint("weather", "chart", |ctx, w, h| Ok(())); // <canvas id="chart"/>, raw D2D
engine.on_moved(|widget, x, y| { /* the user moved a widget */ });
engine.hot_reload();                                   // development: reload on file change
engine.run()?;                                         // message loop until all widgets close
```

| Item | Meaning |
|---|---|
| `Engine::new()` | Creates the engine. Makes the process per-monitor DPI aware (v2). |
| `Engine::load(id, dir, x, y)` | Adds a package widget. `id` must be stable across runs: it keys the saved position and settings. Fails with a message naming the file and line. |
| `Engine::add(id, widget, x, y)` | Adds a widget written in Rust (`impl Widget`). |
| `Engine::set(id, key, value)` | Sets `{app.key}` before start. |
| `widget_engine::set(id, key, value)` | Same, any time while the engine runs (callbacks included). The widget redraws what changed. |
| `Engine::on_action(f)` | `f(widget, action)` for actions the engine does not handle (`open:`, `toggle:`, `set:` are built in). Also `"delete"` when the user deletes the widget from its menu: stop loading it. |
| `Engine::on_paint(widget, canvas, f)` | `f(&ID2D1DeviceContext, w, h)` draws a `<canvas>`, in DIPs. Called on rebuilds and when the canvas's attributes change. |
| `Engine::on_moved(f)` | `f(widget, x, y)` after a move: the card's top-left corner in screen pixels. |
| `Engine::hot_reload()` | Reloads packages when their files change. |
| `Engine::plain_cards()` | Package widgets start with a plain solid card, like gallery previews. Saved user choices still win. |
| `Engine::remember_styles()` | The settings the user picks for a widget become the starting style of the next widget of that package (`open`, `drag_out`). Reset to defaults forgets them. |
| `Engine::keep_alive()` | For tray apps: `run` returns only on `WM_QUIT`, not when the last widget closes. |
| `Engine::run()` | Shows the widgets and runs the message loop. Returns when all widgets are closed. |
| `Engine::start()` / `widget_engine::stop()` | For apps with their own message loop: `start` shows the widgets and returns; your `DispatchMessageW` drives them. |

While the engine runs (from callbacks too; changes apply right after the callback returns):

| Function | Meaning |
|---|---|
| `open(id, dir, x, y)` | Adds a package widget to the desktop. A saved position for `id` wins over (x, y). |
| `open_panel(id, widget, x, y)` | Shows a Rust widget in a panel: a window above the apps (a gallery, a setup screen). A drag on any part outside `Widget::interactive` moves it; Esc closes it. Opening it again brings it to the front. |
| `drag_out(id, dir)` | A new package widget that follows the pointer while the left button is down and lands on the desktop where the user lets go. Call it from a pointer event. |
| `close(id)` | Closes a widget or panel. Its saved position and settings stay. |
| `set_hidden(bool)` | Hides or shows all desktop widgets. Panels stay. |
| `on_paint(widget, canvas, f)` | `Engine::on_paint` for widgets opened later. |
| `popup_menu(x, y, &[MenuItem])` | The engine's own menu at a screen point (e.g. a tray icon). Returns the chosen item's action. `MenuItem::new(label, action)`, `.checked(bool)`, `.danger()`, `MenuItem::submenu(label, items)`. |
| `tr(key)` | An engine string in the Windows display language. |
| `load_image(ctx, bytes)` | Decodes a PNG or JPEG in memory (`include_bytes!`) into a bitmap to draw in `Scene::draw`. |
| `package_info(dir)` | `PackageInfo { id, name, category, size }` of a package, translated, without loading it. |

### Widgets in Rust

For full control, implement `Widget` and draw layers yourself:

```rust
use widget_engine::{Engine, Event, Scene, Widget};

struct Hello;

impl Widget for Hello {
    fn size(&self) -> (f32, f32) { (200.0, 100.0) }
    fn build(&mut self, scene: &mut Scene) -> widget_engine::windows::core::Result<()> {
        let layer = scene.add_layer(0.0, 0.0, 200.0, 100.0)?;
        scene.draw(layer, |ctx| { /* Direct2D */ Ok(()) })
    }
    fn on_event(&mut self, event: Event, scene: &mut Scene) -> widget_engine::windows::core::Result<()> {
        Ok(())
    }
}
```

`Scene` gives layers with their own surfaces (`add_layer`, `draw`), compositor animations
(`rotate`, `spin`, `set_opacity`, `set_scale` with `Transition::Ease` or `Transition::Spring`),
one-shot timers (`set_timer`), the theme, font, and options the user picked.
`Widget::customization` declares what the user may change. See `examples/rust_clock.rs`.

Events: `Timer`, `TimeChanged` (manual change, time zone, DST, resume), `PointerMove`,
`PointerLeave`, `PointerDown`, `PointerUp`, `DataChanged`, `Wheel` (`delta` in notches,
positive away from the user), `Char` (typed text for panels with a text field: `'\u{8}'`
is Backspace, `'\r'` Enter).

`Scene::appear(size, from, how)` plays an opening animation for the whole widget, card included: a fade in while it grows from `from` to full size. For panels.

`Scene::add_preview(dir, x, y, w, h, values)` shows a package widget, scaled into the rect,
as a static picture inside your widget, with `{app.*}` values for the preview. Previews
always use a solid card, whatever the package's background. The widget
gallery uses it for its tiles.

## C

Build `widget.dll`, `widget.dll.lib`:

```bash
cargo build --release -p widget-engine-ffi
```

Include `crates/engine-ffi/include/widget.h` and link `target/release/widget.dll.lib`.

```c
#include "widget.h"

static void on_action(void *user, const char *widget, const char *action) {
    if (strcmp(action, "refresh") == 0) we_set(widget, "peak", "92%");
}

int main(void) {
    WeEngine *engine = we_engine_new();
    if (we_load(engine, "spark", "widgets/sparkline", 100, 100) != WE_OK) {
        fprintf(stderr, "%s\n", we_last_error());
        return 1;
    }
    we_on_action(engine, on_action, NULL);
    return we_run(engine) == WE_OK ? 0 : 1;
}
```

| Function | Rust equivalent |
|---|---|
| `we_engine_new`, `we_engine_free` | `Engine::new`, drop |
| `we_load` | `Engine::load` |
| `we_set` | `widget_engine::set` (works before and after start) |
| `we_on_action`, `we_on_moved` | `Engine::on_action`, `Engine::on_moved` |
| `we_on_paint` | `Engine::on_paint`; `context` is an `ID2D1DeviceContext*` |
| `we_hot_reload` | `Engine::hot_reload` |
| `we_run`, `we_start`, `we_stop` | `Engine::run`, `Engine::start`, `widget_engine::stop` |
| `we_last_error` | Message of the last failed call on this thread |

Return codes: `WE_OK` (0), `WE_INVALID_ARGUMENT`, `WE_LOAD_FAILED`, `WE_RUNTIME_ERROR`,
`WE_NO_WIDGET`. `we_run` and `we_start` take ownership of the engine. Callbacks get a `user`
pointer back. Strings are UTF-8 and valid only during the call.

`examples/c-demo` is a complete app: `build.bat` builds and copies the DLL.

## Persistence

Positions and user settings are saved per instance id in
`%LOCALAPPDATA%\<exe name>\widgets.ini`. The engine clamps saved positions to the work area at
start and after display changes.
