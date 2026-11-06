# Guide: from zero to a widget in Calmdesk

This guide walks through making a widget package, previewing it, feeding it live data from
your app, and shipping it so it shows up in [Calmdesk](https://github.com/rom5n/calmdesk),
the widget gallery for Windows 11. Each step links to the reference docs:
[authoring.md](authoring.md) for packages and [api.md](api.md) for the Rust and C APIs.

## 1. Set up

You need Windows 11, Rust 1.85 or newer, and the MSVC build tools (the "Desktop development
with C++" workload of Visual Studio Build Tools).

```bash
git clone https://github.com/rom5n/widgine.git widgine
```

```bash
cd widgine
```

```bash
cargo run -p widgine --example clock --release
```

A clock appears in the top right corner. Right-click it to see the settings menu users get,
double-click and hold to move it, press Esc on it to close it.

## 2. Create a package

A widget is a folder with a manifest and a layout. `widget-dev new` writes a small clock to
start from:

```bash
cargo run -p widgine --bin widget-dev --release -- new widgets/weather
```

```
widgets/weather/
  widget.toml     id, name, size, card, what users may customize
  layout.xml      what the widget shows
  assets/         images, referenced by relative path
```

Open `widget.toml` and give the package its real identity. The `id` keys every user's saved
settings, so pick it once and keep it:

```toml
id = "com.yourname.weather"
name = "Weather"
category = "Weather"
engine = "^0.1"
size = [170, 170]

[card]
radius = 26
background = "glass"

[customize]
allow = ["size", "background", "theme", "font", "shape", "colors"]
```

## 3. Preview with live reload

```bash
cargo run -p widgine --bin widget-dev --release -- widgets/weather
```

Keep it running. Every save of `widget.toml`, `layout.xml` or an asset redraws the widget.
An error prints with its file and line, and the last good version stays on screen.

Editors complete and check both files: `widget.toml` through the `#:schema` comment
(`schemas/widget.schema.json`), `layout.xml` through `schemas/layout.xsd`.

## 4. Lay it out

Replace `layout.xml` with your design. Rows, columns and stacks lay out children like
flexbox; text and colors follow the user's theme and accent:

```xml
<widget>
  <column padding="18" gap="4" height="fill">
    <text value="{app.city}" size="15" weight="600"/>
    <text value="{app.temp}" size="44" weight="700" features="tnum"/>
    <spacer grow="1"/>
    <text value="{app.sky}" size="13" color="secondary"/>
    <stack padding="4 12" radius="12" fill="#8080802E" hover.fill="#80808050"
           pressed.scale="0.94" transition="spring" action="refresh">
      <text value="Refresh" size="12" weight="600"/>
    </stack>
  </column>
</widget>
```

Things to know while designing:

- Use theme colors (`foreground`, `secondary`, `accent`) instead of fixed ones, so the widget
  reads well in light and dark mode and on every background.
- Elements with `action` take clicks. Everything else is where users double-click and hold to
  move the widget, so leave some of it.
- `{clock.*}`, `{cal.*}`, `{opt.*}`, `{str.*}` and `{app.*}` bindings are listed in
  [authoring.md](authoring.md#bindings).

## 5. Let users choose

Options appear in the widget's right-click menu and bind as `{opt.<key>}`:

```toml
[[option]]
key = "units"
label = "Units"
values = [["c", "Celsius"], ["f", "Fahrenheit"]]
default = "c"

[[option]]
key = "sky"
label = "Conditions"
```

An option without `values` is a Show/Hide toggle: `<text ... if="opt.sky"/>` shows the element
only while it is on.

Translate the name, labels and `{str.*}` texts with `[strings.<language>]` tables, keyed by the
English text. The engine picks the Windows display language:

```toml
[strings.de]
"Weather" = "Wetter"
"Units" = "Einheiten"
"Conditions" = "Wetterlage"
```

## 6. Feed it from your app

Calmdesk shows the widget in its own process. Your app sends values to it with
`widgine::host`; `{app.<key>}` in every widget of the package updates:

```toml
[dependencies]
widgine = "0.1"
```

```rust
use widgine::host;

const PACKAGE: &str = "com.yourname.weather";

fn main() {
    host::on_action(PACKAGE, |widget, action| {
        if action == "refresh" {
            host::set(PACKAGE, "temp", "21°");
        }
    });
    host::set(PACKAGE, "city", "Lisbon");
    host::set(PACKAGE, "temp", "21°");
    host::set(PACKAGE, "sky", "Sunny");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(600));
        // Fetch new data and host::set it again.
    }
}
```

Calmdesk keeps the last values for widgets added later. When Calmdesk starts after your app,
your app sends them again by itself. Clicks arrive on a thread of the engine's.

A working pair to copy: `examples/host-demo` (the package) and
`crates/engine/examples/host_demo.rs` (the app).

From other languages: `we_host_set`, `we_host_on_action` and `we_host_version` in
`crates/engine-ffi/include/widget.h`, or the `WM_COPYDATA` protocol described in
[authoring.md](authoring.md#getting-into-the-calmdesk-gallery).

The values arrive only while your app runs, so give the layout sensible text for empty
values, or hide parts with `if="app.temp"`.

## 7. Try it in Calmdesk

Install [Calmdesk](https://github.com/rom5n/calmdesk/releases), then copy the package folder to
`%LOCALAPPDATA%\Widgets\weather`. Open the gallery from the tray icon: the widget is listed
under its category. Drag it onto the desktop, or click it.

Check before you ship:

- Light and dark theme, and every background in the right-click menu.
- A few sizes, from Small to Huge.
- Empty `{app.*}` values (your app not running).
- The display languages you translated.

## 8. Ship it

Put the package where Calmdesk looks for it, from your app's installer:

- `%LOCALAPPDATA%\Widgets\<package>` for one user, `%ProgramData%\Widgets\<package>` for all.
- Or anywhere, registered as a string value under `HKCU\Software\Widgine\Packages`
  (or `HKLM`), with the package folder as data.
- Or inside your app's own folder in Program Files, `%LOCALAPPDATA%\Programs` or `%APPDATA%`.

Without an installer, ship a zip that users unpack into `%LOCALAPPDATA%\Widgets`. On
uninstall, remove the folder (and the registry value); its widgets leave the desktop at the
next start.

## 9. Host widgets yourself (optional)

An app can also show widgets with its own engine, without Calmdesk:

```rust
use widgine::Engine;

fn main() -> widgine::windows::core::Result<()> {
    let mut engine = Engine::new();
    engine.load("weather", "widgets/weather", 100, 100)?;
    engine.set("weather", "temp", "21°");
    engine.on_action(|widget, action| println!("{widget}: {action}"));
    engine.run()
}
```

Prefer Calmdesk for widgets meant for everyone: users then find all their widgets in one
gallery, with one look and one settings menu. See [api.md](api.md) for the hosting API.
