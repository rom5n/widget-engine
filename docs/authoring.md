# Authoring widget packages

A widget package is a folder:

```
widgets/hello/
  widget.toml     manifest: size, card, font, what users may customize, options
  layout.xml      what the widget shows
  assets/         images (SVG, PNG, JPG), referenced by relative path
```

Start one with `widget-dev new widgets/hello` (a clock to change), then preview it with
live reload while you edit:

```bash
cargo run -p widgine --bin widget-dev --release -- widgets/hello
```

Errors print with the file and line. The last good version stays on screen.
`widget-dev --schema` prints the XML Schema for `layout.xml`.

## widget.toml

```toml
#:schema ../../schemas/widget.schema.json
id = "com.example.hello"      # reverse-domain id
name = "Hello"
variant = "Small"             # optional: tells variants of one widget apart in the gallery
category = "Time"             # gallery section; Time, Media, System, Photos, Productivity and
                              # Weather are translated; empty: "Other"
engine = "^0.1"               # compatible engine versions
size = [300, 150]             # DIPs

[card]                        # engine-drawn background; leave out to draw your own
radius = 24                   # corner radius, the same outline for every background
background = "frosted"        # frosted | glass | solid | clear
border = true                 # hairline edge
shadow = false                # soft drop shadow; the window grows by its size

[font]
family = "Nunito"
weight = 400                  # 1..999
width = 5                     # 1 ultra-condensed .. 9 ultra-expanded

[customize]
# What end users may change in the right-click menu. Empty: right-click shows nothing.
allow = ["size", "background", "theme", "font", "shape", "colors"]

[[option]]                    # widget-specific choice, bound as {opt.range}
key = "range"
label = "Range"
values = [["1h", "Last hour"], ["24h", "Last 24 hours"]]
default = "1h"

[[option]]                    # no values: a Show/Hide toggle with on/off
key = "date"
label = "Date"

[[option]]                    # kind "image" or "folder": the user picks one in a file dialog,
key = "photo"                 # bound as its path ({opt.photo}, "" until picked)
label = "Photo"
kind = "image"

[strings.ru]                  # name, variant, option labels and {str.*} texts in other display
"Range" = "Диапазон"          # languages, by English text
"Last hour" = "Последний час"
```

The menu has **Edit** (the groups below) and **Delete widget**. Engine labels are descriptive
("Small", "Large", "Rounded") and follow the Windows display language.

| `allow` | Menu entries |
|---|---|
| `size` | Small to Huge (75% to 200%). Multiplies the DPI scale, so everything stays sharp. |
| `background` | Material (frosted, liquid glass, solid, transparent), transparency, glass blur and tint (a light veil in light mode, a dark one in dark mode; small by default), background color (14 presets or any color; dark text on light colors). Needs a card. |
| `theme` | System, light, dark. |
| `font` | Family (the fonts in the app's `fonts` folder, grouped Rounded, Modern, Sharp, Techno, Monospace, plus any installed font), weight, width. `[font] family` may name a shipped font too. |
| `shape` | Corner radius, border, shadow. Needs a card. |
| `colors` | Accent color, text color. |
| `text` | Text color only (13 presets or any color), e.g. for widgets without a card. |

Backgrounds:

- **frosted**: the wallpaper and desktop icons behind the card, blurred and saturated.
- **glass**: liquid glass. The backdrop refracts through a convex glass bezel (Snell's law on a
  squircle profile), so content near the edge bends and magnifies, with a little color
  dispersion and a specular rim. The middle stays sharp.
- **solid**: white in light mode, near-black in dark mode, or the user's tint.
- **clear**: a faint tint only.

### Getting into the Calmdesk gallery

Calmdesk lists every package it finds. Install yours to one of these places:

- `%LOCALAPPDATA%\Widgets\<package>` (per user) or `%ProgramData%\Widgets\<package>`
  (all users).
- Anywhere, registered as a string value (any name, the package folder as data) under
  `HKCU\Software\Widgine\Packages` or `HKLM\Software\Widgine\Packages`
  (the older `Software\WidgetEngine\Packages` still works).
- Inside your app's folder in Program Files, `%LOCALAPPDATA%\Programs` or `%APPDATA%`:
  the manager looks a few folders deep for `widget.toml` next to `layout.xml`.

A registered path may use variables such as `%LOCALAPPDATA%`. The gallery shows the app's
folder name as the source.

Calmdesk shows the widget in its own process, so `widgine::set` in your app does not
reach it. Feed it with `widgine::host` instead, from any thread, while your app runs:

```rust
use widgine::host;

host::set("com.example.weather", "temp", "21°"); // {app.temp} in every widget of the package
host::on_action("com.example.weather", |widget, action| println!("{widget}: {action}"));
```

Calmdesk keeps the last values for widgets added later, and your app sends them again
when Calmdesk starts after it. Clicks (`action`, and `drag` releases) arrive on a thread of
the engine's own. `crates/engine/examples/host_demo.rs` feeds `examples/host-demo`.

Apps in other languages speak the same protocol: `WM_COPYDATA` to the window of class
`Calmdesk.Tray`, `dwData` = `0x43444B31`, `lpData` UTF-8 fields separated by U+001F, `wParam`
your window. `set␟<package id>␟<key>␟<value>` sets a value, `hello␟<package id>` asks for the
package's clicks without setting one. Calmdesk sends clicks back to that window as
`action␟<package id>␟<widget id>␟<action>`, and broadcasts the registered message
`Calmdesk.HostStarted` when it starts (your window must be top-level to hear it). `<canvas>`
callbacks do not cross processes: draw with elements and SVG images instead.

Each message is answered with the receiver's protocol version (now 1; 0 means not
delivered), and `host::host_version()` returns Calmdesk's. Later versions only add message
kinds and fields at the end, so ignore kinds you do not know and fields past the ones you
read. An incompatible change would get a new `dwData`.

From C: `we_host_set(package, key, value)`, `we_host_on_action(package, f, user)` and
`we_host_version()` in `widget.h` do the same, no engine needed.

### Publishing a package

1. Give it a unique reverse-domain `id` (`com.yourname.weather`); it keys users' saved
   settings, so keep it across versions.
2. Check it: `widget-dev <dir>` prints any error with file and line. Try light and dark
   themes and a few sizes from the right-click menu.
3. Ship it with your app's installer into one of the places above, or as a zip users unpack
   into `%LOCALAPPDATA%\Widgets\<package>`. Calmdesk finds it the next time its gallery opens.
4. Uninstall: remove the folder (and the registry value if you added one). Widgets of a
   package that is gone disappear from the desktop at the next start.
5. If your app feeds the widget, it does so only while it runs: give the layout sensible
   text for empty `{app.*}` values.

## layout.xml

```xml
<widget xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
        xsi:noNamespaceSchemaLocation="../../schemas/layout.xsd">
  <style class="pill" padding="6 14" radius="14" fill="#FFFFFF1F" hover.fill="#FFFFFF33"/>
  <column padding="18 20" gap="10">
    <text value="{clock.time}" size="48" features="tnum"/>
    <stack class="pill" action="toggle:range">
      <text value="{opt.range}" size="13"/>
    </stack>
  </column>
</widget>
```

### Elements

| Element | Own attributes |
|---|---|
| `row`, `column` | Panel attributes (below). Children side by side or stacked. |
| `stack` | Panel attributes. Children on top of each other, centered by default. |
| `rect` | Panel attributes. A shape that fills its box. |
| `text` | `value`, `size`, `weight`, `family`, `color`, `text-align` (start, center, end), `tracking` (letter spacing), `features` (OpenType, e.g. `tnum ss01`), `ellipsis="true"` (one line, cut with … to fit its box; give it `width="fill"` or `grow`). |
| `image` | `src`: SVG or raster file in the package, or an absolute path from a binding (`src="{app.art}"`). SVG files may use bindings and `{accent}`, `{foreground}`, `{secondary}`. Raster images cover their box (centered crop). `radius`: continuous rounded corners; with `corners="round"` plain ones (a circle when the radius is half the size). |
| `ellipse` | `fill`, `stroke`, `stroke-width`. |
| `canvas` | `id`, `value` (any binding; the canvas redraws when it changes). Drawn by the app, see `Engine::on_paint`. |
| `spacer` | Empty box. With `grow="1"` it takes the free space in a row or column. |

Panel attributes: `fill`, `stroke`, `stroke-width`, `radius`, `corners="round"` (plain round
corners instead of continuous ones).

Every element also takes:

| Attribute | Meaning |
|---|---|
| `width`, `height` | A number in DIPs, or `fill`. Default: the content size. |
| `grow` | Share of the free space in a row or column. |
| `padding` | `"all"`, `"vertical horizontal"`, or `"top right bottom left"`. |
| `gap` | Space between children. |
| `align`, `justify` | `start`, `center`, `end`: cross axis and main axis. |
| `if` | `opt.date` or `!opt.date`: show the element only when the value is on / not empty. |
| `class` | Style classes, `class="a b"`. The element's own attributes win. |
| `opacity`, `scale` | Run on the compositor. Opacity multiplies down the tree. |
| `rotate`, `spin` | Degrees clockwise; `spin` in degrees per second (compositor animation). |
| `shadow` | `"dx dy blur color, ..."`, layered drop shadows. |
| `transition` | `"200"` (ms, ease-out) or `"spring"` for changes of `opacity` and `scale`. |
| `action` | Fired on click, see below. |
| `drag` | A slider: `drag="seek"` binds `{drag.seek}` (0-100, where the pointer is across the element) while held, and fires the action `seek:<0-1>` on release. Hover and pressed states stay on while it is held. |

### Colors and paints

`#RGB`, `#RRGGBB`, `#RRGGBBAA`, or theme tokens `foreground`, `secondary`, `accent`,
`transparent`. They follow light/dark mode and the user's accent and text colors.
`fill`, `stroke` and text `color` also take gradients:

```
linear(135deg, #FF5F6D, #FFC371)
radial(#FFFFFF40, transparent)
conic(0deg, #FF3B30, #FFCC00 50%, #FF3B30)
```

### States

`hover.<attr>` and `pressed.<attr>` override any attribute that does not move things:
`hover.fill="#FFFFFF33" pressed.scale="0.94"`. Add `transition` to animate opacity and scale.

### Bindings

Any attribute may contain `{name}`:

| Provider | Values |
|---|---|
| `clock` | `time`, `time_seconds`, `date`, `weekday`, `weekday_short`, `day`, `month`, `month_short`, `year`, `hour`, `minute`, `second`, `hour_angle`, `minute_angle`, `second_angle`. Locale formats. Updates every minute, or every second when a second is shown. |
| `cal` | Month grid: `wd0`..`wd6` weekday initials from the user's first day of the week, `d0`..`d41` day numbers ("" outside the month), `t0`..`t41` today flags, `c0`..`c41` color tokens (today, weekend, normal), `we0`..`we6` weekend columns, `row5` (6th week used), `month` (capitals). See `widgets/calendar`. |
| `opt` | The widget's options from `widget.toml`. |
| `str` | `{str.Battery}`: a text of the `[strings.*]` tables in the display language, the English key when there is none. |
| `theme` | `theme.dark`: on/off. |
| `app` | Values the app sets with `Engine::set` / `we_set`. |

### Actions

| Action | Effect |
|---|---|
| `open:<url or path>` | Opens it with the default app. |
| `toggle:<option>` | Next value of an option (on/off for toggles). |
| `set:<option>=<value>` | Sets an option. |
| anything else | Goes to the app's `on_action` callback, then the widget redraws. |

Clickable parts take clicks. Double-click and hold anywhere else moves the widget.
