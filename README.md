# widget-engine

GPU-rendered desktop widgets for Windows 11. Your app embeds the engine and shows its own
widgets on the desktop: below every window, next to the icons, at about 0% CPU while idle.

![The clock, calendar and sparkline examples in the light and dark themes](docs/images/examples.png)

- **Native rendering:** D3D11, Direct2D, DirectWrite and DirectComposition through `windows-rs`.
  No WebView, no UI framework. Animations run in the compositor.
- **Declarative packages:** a widget is a folder with `widget.toml`, `layout.xml` and assets.
  Flexbox-style layout, bindings, gradients, layered shadows, hover/pressed states, springs.
- **Full design control:** rounded frosted and liquid-glass cards that
  show the wallpaper and desktop icons behind them, plus a `<canvas>` with the raw
  `ID2D1DeviceContext` for anything else.
- **User customization:** right-click opens a settings menu with exactly what the developer
  allows: size, background, corners, shadow, theme, fonts, colors, and widget options.
  Widgets with nothing to customize show no menu.
- **Desktop behavior:** double-click and hold to move a widget, snapping to screen edges and
  other widgets, kept inside the work area, positions saved per instance. Widgets hide while a
  fullscreen app runs and survive Explorer restarts, DPI changes and GPU resets.
- **Rust and C APIs:** the `widget-engine` crate, and `widget.dll` + `widget.h` for any language.

The same clock package in every background, light theme on top and dark below: frosted,
liquid glass, solid and clear.

![The clock example in the frosted, liquid glass, solid and clear backgrounds](docs/images/materials.png)

## Quick start (10 minutes)

The step-by-step [guide](docs/guide.md) goes from an empty folder to a widget your app feeds
and ships into [Calmdesk](https://github.com/rom5n/calmdesk).

Needs Windows 11, Rust 1.85+ and the MSVC build tools.

```bash
git clone https://github.com/rom5n/widget-engine.git widget && cd widget
cargo run -p widget-engine --example clock --release
```

The clock appears in the top right corner of your desktop. Right-click it for settings,
double-click and hold to move it, press Esc on it to quit. More examples:

```bash
cargo run -p widget-engine --example calendar --release
```

```bash
cargo run -p widget-engine --example sparkline --release
```

### Your own widget

1. Copy `examples/widgets/clock` to `widgets/hello` and open it with live reload:

   ```bash
   cargo run -p widget-engine --bin widget-dev --release -- widgets/hello
   ```

2. Edit `widgets/hello/layout.xml` and save. The widget updates on the desktop:

   ```xml
   <widget>
     <column padding="20" gap="4">
       <text value="Hello, {app.name}" size="28"/>
       <text value="{clock.weekday}, {clock.time}" size="14" color="secondary"/>
       <stack padding="6 14" radius="14" fill="accent" hover.scale="1.05" transition="spring"
              action="wave">
         <text value="Wave" size="13" color="#FFFFFF"/>
       </stack>
     </column>
   </widget>
   ```

3. Host it in your app:

   ```rust
   use widget_engine::Engine;

   fn main() -> widget_engine::windows::core::Result<()> {
       let mut engine = Engine::new();
       engine.load("hello", "widgets/hello", 100, 100)?;
       engine.set("hello", "name", "World");
       engine.on_action(|widget, action| println!("{widget}: {action}"));
       engine.run()
   }
   ```

Editors pick up `schemas/widget.schema.json` (TOML, via the `#:schema` comment) and
`schemas/layout.xsd` (XML) for completion and errors as you type.

### From C

```c
#include "widget.h"

WeEngine *engine = we_engine_new();
we_load(engine, "clock", "examples/widgets/clock", 100, 100);
we_set("clock", "label", "Tokyo");
we_run(engine);
```

`examples/c-demo/build.bat` builds a complete C example against `widget.dll`.

## Docs

- [docs/guide.md](docs/guide.md): step by step, from a new package to shipping it in Calmdesk.
- [docs/authoring.md](docs/authoring.md): widget packages, layout elements, bindings, styling.
- [docs/api.md](docs/api.md): Rust and C APIs.
- [docs/rendering.md](docs/rendering.md): how the engine works inside, for contributors.
- [PLAN.md](PLAN.md): scope, milestones, budgets.

## Layout of this repository

| Path | What |
|---|---|
| `crates/engine` | The engine crate, the `widget-dev` previewer, Rust examples |
| `crates/engine-ffi` | C ABI: `widget.dll`, `include/widget.h` |
| `examples/widgets` | Packages the examples show: clock, calendar, sparkline |
| `examples/c-demo` | C app hosting two widgets |
| `examples/host-demo` | Package fed through `widget_engine::host` while [Calmdesk](https://github.com/rom5n/calmdesk) shows it (`host_demo` example) |
| `schemas/` | JSON Schema for `widget.toml`, XML Schema for `layout.xml` |

## Calmdesk

[Calmdesk](https://github.com/rom5n/calmdesk) is the desktop widget app built on this engine. It
shows every widget package it finds on the PC; see "Getting into the Calmdesk gallery" in
[docs/authoring.md](docs/authoring.md).

## License

MIT, see [LICENSE](LICENSE).
