# MLP plan: Windows 11 widget engine

**Goal:** a developer clones the repo, runs `cargo run --example clock`, and sees a clock on their desktop that looks native to Windows 11 and uses ~0% CPU while idle. Within 10 minutes they have their own widget running inside their own app.

The deliverable is an **engine** (a library), not an end-user app. There is no installer, tray app, or widget manager. Each app embeds the engine and shows its own widgets. The clock is the reference example.

## Scope

**In:**
- Engine library: windows, rendering, declarative layout, bindings, themes, hot reload.
- Public API: Rust crate + C ABI (`widget.dll` + `widget.h` + `widget.lib`), so any language can use it.
- Example: **Clock** widget package and the example apps that host it (Rust and C).
- Dev tool: `widget-dev <path>` previewer with hot reload.
- Docs and a GitHub release of the SDK binaries.

**Out (post-MLP):** shared host process across apps, WASM logic, .NET binding, more example widgets, gallery/store, installer, permission prompts.

## Architecture decision: embedded, in-process

The engine runs inside the app that uses it. The app calls the API directly, with no IPC and no separate host process.

- Pro: simplest model. Widget lifetime = app lifetime. No protocol to version.
- Con: two apps using the engine each pay their own engine overhead (~10–20 MB). Acceptable for MLP. A shared host can come later behind the same API.

## Repo layout

```
widget/
  crates/
    engine/       core: windows, rendering, layout, bindings, themes
    engine-ffi/   C ABI over engine -> widget.dll + widget.h
    widget-dev/   previewer with hot reload: widget-dev <path>
  widgets/
    clock/        widget.toml, layout.xml, assets/
    calendar/     macOS-style month view (cal.* provider)
    sparkline/    canvas, glass, app values
    engine/examples/
      clock.rs    Rust app hosting the clock (cargo run -p widget-engine --example clock)
      spike.rs    M0 spike, kept for backdrop experiments
  examples/
    c-demo/       C app linking widget.dll
  schema/         JSON Schema for widget.toml + layout.xml (editor autocomplete)
  docs/           authoring.md, api.md, rendering.md
  .github/workflows/ci.yml, release.yml
```

## Public API (MLP shape)

```rust
let engine = widget::Engine::new()?;
let clock = engine.load("widgets/clock")?;        // widget package directory
clock.set("label", "Tokyo")?;                     // app-supplied binding value
clock.on_action("tap", |_| println!("tapped"))?;  // action from layout.xml
engine.run();                                     // or engine.pump() inside the app's own message loop
```

- The engine creates its windows on the calling thread. Apps with an existing message loop call `pump()`. Apps without one call `run()`.
- The C ABI mirrors this API one-to-one with opaque handles and `int` error codes.
- Widget position and user settings: the engine handles drag and the settings menu, and persists both per instance id (see User customization).

## Rendering

Native Windows GPU APIs via `windows-rs`. No WebView, no UI framework, no third-party graphics library.

### Stack

| Layer | Role |
|---|---|
| **D3D11** | One device per engine (`D3D11_CREATE_DEVICE_BGRA_SUPPORT`). Only the GPU handle, no 3D code. |
| **Direct2D 1.1+** | Shapes, gradients, rounded rects, images, the analog clock face. Draws via `ID2D1DeviceContext`. |
| **DirectWrite** | Text layout and rendering, Segoe UI Variable with variable font axes, emoji, RTL. |
| **WIC** | PNG/JPG decode into cached D2D bitmaps. SVG via `ID2D1DeviceContext5::CreateSvgDocument`. |
| **DirectComposition** | Retained visual tree. DWM composites it and runs animations. |
| **DWM** | Final composition. Windows use `WS_EX_NOREDIRECTIONBITMAP`, so there is no GDI redirection surface. |

### How a frame works

1. Startup: create the D3D11 device, then the D2D device from its `IDXGIDevice`, then `DCompositionCreateDevice2` with the D2D device. All widgets in the process share these devices.
2. The layout tree maps to a visual tree. Every element that changes independently gets its own `IDCompositionVisual` with its own `IDCompositionSurface`. Static elements are merged into one surface.
3. A binding change marks only its element dirty. The engine calls `IDCompositionSurface::BeginDraw` for that surface only, draws with the D2D context it returns, then `EndDraw`.
4. All changes in one update are batched into a single `Commit()`.
5. Between updates the engine does nothing: no render loop, no timer, no polling.

### Animation

- Transforms, opacity, and rotation use `IDCompositionAnimation`. DWM runs them at the display refresh rate with zero CPU in the app.
- Clock second hand: a linear 0→360° rotation over 60 s, looped. It is re-synced to the system clock once per minute and after time-change or resume events.

### Details that matter

- **Alpha:** surfaces use `DXGI_ALPHA_MODE_PREMULTIPLIED`, so widgets can have transparent rounded corners and translucency.
- **Text antialiasing:** ClearType needs an opaque background, so text on translucent surfaces uses grayscale antialiasing. Text on opaque surfaces may use ClearType.
- **DPI:** the app is per-monitor DPI aware v2. Layout uses DIPs, and surfaces are re-rasterized on `WM_DPICHANGED`. There is no bitmap scaling.
- **Device lost:** on `DXGI_ERROR_DEVICE_REMOVED`, or when `IDCompositionDevice::CheckDeviceState` reports a lost device, recreate all devices and surfaces, then redraw everything. Driver updates and GPU resets must not kill widgets.
- **Caches:** `IDWriteTextLayout` is cached per text element and rebuilt only when its text or style changes. Bitmaps are cached per asset path.
- **Backdrop:** chosen in the M0 spike (see milestones). The fallback is a solid surface tinted with the accent color.

### Why not the alternatives

| Option | Reason rejected |
|---|---|
| WebView2 | 100+ MB per process, a Chromium runtime, idle power drain. |
| WinUI 3 / XAML | 50–80 MB, its own window model, fights borderless desktop-pinned windows. |
| Skia / wgpu / vello | Larger binary, weaker text than DirectWrite, still needs DComp for composition. |
| egui / iced / Slint | Their own render loops. Hard to reach ~0 CPU at idle. |

## Clock widget spec (reference example)

- **Digital variant:** large `HH:mm` in Segoe UI Variable Display, with a weekday and date line under it.
- **Analog variant:** D2D face and hands. The second hand uses the DComp rotation animation described above.
- **Locale:** 12h/24h and date format follow the system (`GetLocaleInfoEx`, `GetDateFormatEx`), with a per-instance override.
- **Options:** show seconds, show date, time zone plus label. Multiple instances give a world clock.
- **Correct timing:**
  - A timer is aligned to the next minute boundary, or to the next second when seconds are shown in the digital variant.
  - Handle `WM_TIMECHANGE`, DST transitions, and resume from sleep (`WM_POWERBROADCAST`).
- **Look:** accent color, light/dark follow, rounded corners, translucent backdrop.
- **Sizes:** small (2×1), medium (2×2), wide (4×2).
- **Built from engine features only:** the clock package contains no Rust code. Time values come from a built-in `clock` binding provider.

## Design: full control for developers

Developers get full control over how a widget looks, so widgets can match the polish of macOS widgets. Control comes in two levels.

**Level 1: declarative styling** in `layout.xml`. This covers almost every design.

| Area | Capabilities |
|---|---|
| Shape | Per-corner radius, **continuous corners** (squircle, `corner-smoothing="0.6"`, the macOS look), SVG paths, clipping to any shape. |
| Fill | Solid, linear, radial, and conic gradients with any number of stops, image fill. |
| Border | Width, solid or gradient stroke, inside/center/outside alignment. |
| Shadow | Multiple layered drop shadows, inner shadow, colored shadows. |
| Material | Backdrop blur, tint color and opacity, saturation, noise/grain overlay (vibrancy look). |
| Effects | Blur, color matrix, blend modes, masks, opacity. |
| Typography | Bundled font files (custom DirectWrite font collection), variable axes (`wght`, `opsz`, ...), OpenType features (`tnum`, `ss01`, ...), letter spacing, line height, gradient text fill, text shadow. |
| Motion | Spring and cubic-bezier animations, keyframes, transitions on value change (crossfade, numeric roll, slide). Run on the compositor where possible. |
| States | `hover`, `pressed` styles with transitions. |
| Reuse | Named style classes in a `<styles>` block, theme tokens for light/dark. |

**Level 2: `canvas` element** as an escape hatch. A `<canvas id="face"/>` element gets a paint callback with the raw `ID2D1DeviceContext`. The developer draws anything D2D can draw, including D2D effects. The engine still handles DPI, dirty tracking, caching, and composition.

```rust
clock.on_paint("face", |ctx: &ID2D1DeviceContext, size| { /* any D2D drawing */ })?;
```

The C ABI passes the raw COM pointer, so C/C++ developers get the same power.

Implementation notes:
- Continuous corners are built as a D2D path geometry (superellipse-style curve), cached per size and radius.
- D2D has no conic gradient brush, so conic gradients are drawn with a custom D2D effect (pixel shader) or a cached mesh.
- Blur, shadow, and color matrix use built-in D2D effects (`CLSID_D2D1GaussianBlur`, `CLSID_D2D1Shadow`, `CLSID_D2D1ColorMatrix`). Results are cached per surface and re-rendered only when the input changes.
- Spring animations are sampled into cubic segments so DComp can run them without CPU work.

## User customization

End users change a widget's look without code, but only the parts the developer opened up. The developer opts in per widget, in code now (`Widget::customization`) and in `widget.toml` once M2 lands.

**Access:** right-click the widget. A widget that allows nothing (no `allow` entries, no options) shows no menu at all. A native Windows 11 context menu shows the available settings and follows dark mode. Later (M3) an engine-drawn settings panel adds live preview, sliders, and a color picker.

| Group | Settings | Status |
|---|---|---|
| Size | 75%, 100%, 125%, 150%, 200%. Multiplies the DPI scale, so everything stays sharp. | M1 |
| Background | Material: Solid, Frosted (blurred wallpaper), Clear. Tint color. Opacity. | M1 |
| Background | Liquid glass: the backdrop (wallpaper and desktop icons) refracts through a convex squircle bezel (Snell's law, n = 1.5), dispersion, specular rim. CPU lens, see M2/M3. | M2, M3 done |
| Shape | Corner radius, border on/off, shadow. | M2 done |
| Theme | System, Light, Dark. Accent color override. | M1 (theme), M2 done (accent) |
| Text | Font family (curated list plus any installed font), weight, width (for fonts that have widths, e.g. Bahnschrift), text color. | M1 (family, weight, width), M2 done (color) |
| Widget options | Choices declared by the developer, e.g. clock: second hand, date, dial. | M1 |
| Behavior | Reset to defaults, close. Moving: double-click and hold, then drag (a single press never moves). Lock position was removed in M3. | M1, M3 |

**Persistence:** the engine stores position and settings per widget instance in `%LOCALAPPDATA%\<app>\widgets.ini`. The app gives each instance a stable id in `Engine::add`.

**Stays on screen:** while dragging, the widget is kept inside the work area of its monitor. Saved positions are clamped at start and after display changes.

## Engine features

- **Elements:** `stack`, `row`, `column`, `text`, `image`, `shape`, `progress`, `canvas`. A flex subset, not CSS. Styling as described in the design section.
- **Bindings:** `{clock.time}` from built-in providers, `{label}` from app values set through the API.
- **Actions:** `on-tap="name"` in layout.xml fires the app callback registered with `on_action`.
- **Themes:** tokens (accent, foreground, surface) that follow light/dark mode and the accent color via `UISettings`.
- **Window behavior:** borderless, desktop-level placement, drag to move, snap to grid, multi-monitor, hide when a fullscreen app runs (`SHQueryUserNotificationState`), pause rendering when occluded or the session is locked.
- **Hot reload:** `widget-dev <path>` watches `layout.xml` and `widget.toml` and reloads on save.
- **Schema:** JSON Schema in `schema/`, so editors give autocomplete and validation.

## Performance budgets (CI fails if exceeded)

| Metric | Budget |
|---|---|
| Idle CPU, clock example | ~0.0% average over 60 s |
| Engine RAM overhead, one clock | < 20 MB |
| `Engine::new()` to first frame | < 150 ms |
| Clock tick work | 1 dirty surface redraw per minute (digital, no seconds) |
| Animations | Compositor only, no CPU per frame |
| `widget.dll` size | < 3 MB |

## Milestones (solo dev, about 7 weeks)

**M0: Spikes (week 1).** De-risk the hard platform parts first. Code: `crates/engine/examples/spike.rs`.
- [x] Minimal render path: D3D11 + D2D + DComp window with `WS_EX_NOREDIRECTIONBITMAP`, DirectWrite text on a premultiplied-alpha surface, second hand rotated by a DComp animation. Idle CPU ~0%.
- [x] Desktop placement. Decision: **child window of the icon host (`Progman` on 24H2+), above `SHELLDLL_DefView`**. Findings on build 26300:
  - Child of the wallpaper `WorkerW`: invisible, because WorkerW paints over its children.
  - Child of `Progman` below the icons: visible only with `WS_EX_LAYERED`, but the full-screen icon layer takes all mouse input.
  - Child of `Progman` above the icons: visible, draggable, gets focus on click. Covers icons it overlaps, so the frosted and glass cards draw the icons themselves (M3: `PrintWindow` capture of the icon list view, refreshed by a WinEvent hook).
  - Child windows get no broadcasts (theme, time change, power, `TaskbarCreated`), so the engine needs a hidden top-level window.
  - Not tested yet: Win+D, virtual desktops.
- [ ] Backdrop. System backdrops (Mica/Acrylic/accent blur) work only on top-level windows, so they don't apply in the chosen placement. Behind a desktop-layer widget there is only the wallpaper, so the plan is: load the wallpaper (`IDesktopWallpaper`), blur it once with D2D, and draw the part under the widget. Redo only when the wallpaper or widget position changes. Until then: solid squircle surface.
- [ ] Memory: 52–60 MB private with one widget vs the 20 MB budget (76 MB with the frosted card: WIC, Shell, effect DLLs). Most likely the GPU driver's allocations for the D3D11 device. Measure an empty device, then decide between optimization and a new budget.

**M1: Rendering core + hardcoded clock (week 2).**
- [x] Cargo workspace, `engine` crate: shared GPU devices (`gpu.rs`), layers with their own surfaces (`scene.rs`), one commit per update.
- [x] Desktop placement and hidden host window for broadcasts (`desktop.rs`, `lib.rs`).
- [x] Device-lost recovery: the driver signals an event (`RegisterDeviceRemovedEvent`), the loop wakes and rebuilds everything. No polling.
- [x] DPI: per-monitor scale, rebuild after a drag to another monitor or a display change.
- [x] Explorer restart: widget windows die with Explorer and are recreated on `TaskbarCreated`.
- [x] Clock as a Rust widget on the engine API (`crates/engine/examples/clock.rs`): minute-aligned timer, time-change and resume handling, light/dark, accent.
- [x] Unit tests for the timer math (`time.rs`).
- [ ] Verify by hand: forced device removal (`dxcap -forcetdr`), Explorer restart, drag to a second monitor with a different DPI.
- [ ] Exit: the clock runs for 24 h correctly. Idle CPU measured: 15.6 ms over 45 s, minute tick included.

**M2: Declarative engine + design system (weeks 3–5).**
- [x] Parsers for `widget.toml` (serde, unknown keys rejected) and `layout.xml` (tags and attributes validated, errors name the line). Engine version check.
- [x] Layout engine: flexbox subset (`row`, `column`, `stack`, `spacer`; `grow`, `fill`, `padding`, `gap`, `align`, `justify`).
- [x] Bindings: `{clock.*}`, `{opt.*}`, `{theme.dark}` in attributes and inside SVG files; `{accent}`, `{foreground}`, `{secondary}` colors in SVG. `if="opt.x"` / `if="!opt.x"`.
- [x] Elements: `text` (tracking, OpenType features), `rect` / panels (fill, stroke, continuous or round corners), `ellipse`, `image` (SVG or raster), `opacity`, `rotate`, `spin` (compositor animation).
- [x] Style classes: `<style class="name" .../>` in `<widget>`, `class="a b"` on elements, own attributes win.
- [x] Gradients for `fill`, `stroke`, text `color`: `linear(deg, ...)`, `radial(...)`, `conic(deg, ...)` (conic via a CPU bitmap).
- [x] Layered drop shadows: `shadow="dx dy blur color, ..."` (D2D Shadow effect, layer grows by the shadow margin). Opacity moved to the compositor.
- [x] Actions: `action="name"` on any element. Built in: `open:<url or path>`, `toggle:<option>`, `set:<option>=<value>`; the rest goes to `Engine::on_action(|widget, action|)`, then the widget redraws. Clickable parts get `HTCLIENT`, the rest of the widget stays a drag handle. `Scene::action` for Rust widgets.
- [x] States: `hover.<attr>` / `pressed.<attr>` for every non-layout attribute. Pointer events (`Event::PointerMove/Leave/Down/Up`, `Widget::interactive`). Pointer moves that change no state skip the layout pass.
- [x] Motion: `transition="200"` (ease-out cubic) or `"spring"` for `opacity` and the new `scale`, run by DComp animations (springs sampled into 1/60 s segments); a new target starts from the current animated value. Opacity multiplies down the tree, scale applies to the element and all inside it.
- [x] `<canvas id="..."/>` with `Engine::on_paint(widget, canvas, |ctx, w, h|)`, demo in `widgets/sparkline` + `examples/sparkline.rs`.
- [x] Materials: liquid glass card (`background = "glass"`): wallpaper read back at 1/2 size, lens bend at the rounded rim with color fringing, specular rim. ponytail: rendered on the CPU per card redraw (~1 ms), a D2D pixel shader effect if it ever animates.
- [x] Clock rewritten as the package `widgets/clock/` (digital + analog dial, options: dial, hands, second hand, date). The Rust-API clock stays as `examples/rust_clock.rs`.
- [x] `widget-dev <package-dir>` with hot reload (`Engine::hot_reload`, polls file times every 300 ms; errors print with file and line, the last good version stays). JSON Schema for `widget.toml` in `schemas/` (`#:schema` comment).
- [x] XML schema `schemas/layout.xsd`, generated from the parser's own tables (`widget-dev --schema`); a test fails when it is stale.
- [x] User settings: Liquid glass background, Corners (radius), Border, Shadow (`allow = ["shape"]`), Accent color, Text color (`allow = ["colors"]`). The card shadow grows the window by its margin; saved positions stay the card's corner.
- [x] Exit: the clock package contains no Rust code. Measured: 50 MB working set, 15.6 ms CPU over 30 s. Sparkline demo (glass, shadow, canvas): 73–82 MB, 15.6 ms CPU over 15 s idle.
- Not done in M2: keyframes, numeric roll / slide transitions, transitions for colors and other redrawn attributes (they switch instantly), inner shadows, blend modes, masks, bundled font files, variable font axes, line height, per-corner radius, image fill, gradient/inside/outside border alignment, noise overlay. Moved to the backlog.

**M3: Public API + window behavior (week 6).**
- [x] Rust API: `Engine::load`, `Engine::set` / `widget_engine::set` (`{app.key}` bindings, live), `on_action`, `on_paint`, `on_moved` (position callback), `run`, and `start` / `stop` for apps with their own message loop (their `DispatchMessageW` drives the widgets, so no `pump` is needed).
- [x] Move by double-click and hold (single presses never move); snapping to work-area edges and other widgets (12 DIPs, 12 DIP gap), computed from the drag start so a slow drag lets go; kept on screen; multi-monitor DPI.
- [x] Fullscreen hide: the host registers as a shell appbar only to get `ABN_FULLSCREENAPP`; widgets hide while a fullscreen app runs and refresh after.
- [x] Right-click shows no menu when the package allows nothing; the Lock position entry is gone.
- [x] Desktop icons visible behind frosted and glass cards: the icon list view is captured with `PrintWindow(PW_RENDERFULLCONTENT)` (alpha from brightness, it renders over black) and composed over the wallpaper; a WinEvent hook on the list view recaptures 400 ms after icons change. No polling.
- [x] Liquid glass rebuilt after kube.io's "Liquid Glass in the Browser": convex squircle height profile, Snell refraction (n = 1.5) precomputed per bezel distance, sampled inward (magnifies the edge), per-channel dispersion, steepness-weighted rim light. Full-resolution backdrop so the bending is visible.
- [x] `widgets/calendar` (macOS-style month widget) on a new built-in `cal.*` provider (locale first weekday and initials, 6x7 grid, today, weekend colors). Reviewed by a design agent against the macOS Calendar widget: 8/10, then its top fixes applied (white light card, bold today, balanced rows). `examples/calendar.rs`.
- [x] `examples/clock.rs`, `calendar.rs`, `sparkline.rs` (now also shows `{app.peak}`).
- [x] `crates/engine-ffi`: C ABI `widget.dll` (929 KB), hand-written `include/widget.h` (cbindgen not installed; the C example compiling at `/W4` checks it), `examples/c-demo/` (clock + sparkline, `we_set` from an action callback). Verified end to end.
- Not done: occlusion and session-lock pause. Widgets sit under every window and do one redraw per minute, so the saving would be near zero; add `WTSRegisterSessionNotification` if a widget ever animates continuously. `start` mode notices GPU loss on the next draw error, not instantly.
- Measured: clock and calendar 52 MB, sparkline (full-res glass) ~100 MB; idle 16 ms CPU per 20 s each.

**M4: Docs + release (week 7).**
- [x] README with a screenshot (`docs/images/widgets.png`; a GIF still to record) and a 10-minute quick start.
- [x] `docs/authoring.md` (widget packages), `docs/api.md` (Rust + C), `docs/rendering.md` (internals for contributors).
- [x] CI (`.github/workflows/ci.yml`): build, tests (schema check included), `widget.dll` size budget, C example at `/W4 /WX`. Idle CPU and RAM need a desktop session, which hosted runners lack: measured by hand.
- [x] `release.yml`: on a `v*` tag, builds `widget-sdk-x.y.z-x64.zip` (dll, lib, header, `widget-dev.exe`, clock and calendar packages, schemas, docs), a GitHub release, and `cargo publish`.
- [ ] v0.1.0 release: needs the owner. The folder is not a git repo yet; init, push to GitHub, add the `CARGO_REGISTRY_TOKEN` secret, then tag `v0.1.0`.

## Definition of done

1. Fresh clone, `cargo run --example clock`: the clock appears on the desktop.
2. `widget-dev widgets/clock`: editing `layout.xml` updates the clock live.
3. A C app links `widget.dll`, loads a package, sets a value, and receives a tap action.
4. All performance budgets are green in CI.

## Versioning

- Semver for the Rust API, the C ABI, and the `widget.toml`/`layout.xml` schema. The manifest field `engine = "^0.1"` declares compatibility.
- A breaking change to the C ABI or the schema requires a major version bump after 1.0.

## Risks

| Risk | Mitigation |
|---|---|
| Inactive-window backdrop fallback kills the frosted look | M0 spike first. Fallback: solid surface with accent tint. |
| Desktop pinning breaks after Win11 updates | Use the documented bottom-z approach. Avoid undocumented tricks. |
| Device lost on driver update kills widgets | Recovery path built in M1 and tested with forced TDR. |
| Embedded model duplicates overhead across apps | Accepted for MLP. A shared host can be added later behind the same API. |
| Scope creep | Ship only the engine and the clock example. |
