# Rendering internals

For contributors. Code lives in `crates/engine/src`.

## Devices (`gpu.rs`)

One set per process: a D3D11 device (BGRA support, no 3D code), a Direct2D device and context
on its DXGI device, DirectWrite, and a DirectComposition desktop device. The driver signals GPU
loss through `RegisterDeviceRemovedEvent`; `run` waits on that event next to the message queue,
then recreates every device, target and scene.

## Windows (`desktop.rs`)

A widget is a child window of the window that hosts the desktop icons (`Progman` on 24H2+),
placed above `SHELLDLL_DefView`. It is created as a popup and then turned into a
`WS_EX_LAYERED` child with `WS_EX_NOREDIRECTIONBITMAP`: DWM composites its DComp tree and
there is no GDI surface. Child windows get no broadcasts, so a hidden top-level host window
receives theme, time, power, display, `TaskbarCreated` (Explorer restart) and appbar
notifications (fullscreen apps) and owns menus and timers.

M0 findings on build 26300: under the wallpaper `WorkerW` a widget is invisible; under the
icon layer it is visible but gets no input. Hence "above the icons", and the cards draw the
icons themselves (below).

Input: the client area takes all mouse input (`WM_LBUTTON*`, `WM_MOUSEMOVE`,
`TrackMouseEvent`). A double-click outside the widget's clickable parts hands the held button
to the system move loop (`WM_NCLBUTTONDOWN` + `HTCAPTION`). `WM_MOVING` snaps the card to
the work area edges and other widgets and keeps it on screen; the position is saved on
`WM_EXITSIZEMOVE`.

## Scenes and layers (`scene.rs`)

Each widget has a `Scene`: a root `IDCompositionVisual2` and layers, each with its own
`IDCompositionSurface`. Drawing a layer is `BeginDraw` on that surface only, then `EndDraw`.
All changes of one update go out in one `Commit`. Opacity, scale and rotation are DComp
properties and animations: DWM runs them at the display rate with no CPU work in the app.
Springs (stiffness 300, damping 20) are sampled into 1/60 s linear segments; a retarget
starts from the current animated value.

Nothing runs between updates. A package widget asks for one timer at the next minute (or
second) boundary.

## Markup (`package.rs`, `layout.rs`, `markup.rs`)

`package.rs` parses `widget.toml` (serde, unknown keys rejected) and `layout.xml` (roxmltree;
every element and attribute checked against one table, which also generates
`schemas/layout.xsd`). `layout.rs` is a flexbox subset. `markup.rs` turns laid-out nodes into
layers. Each drawn element keeps a signature of its resolved attributes; a tick redraws only
the layers whose signature changed, and a change of layout boxes rebuilds the scene. Pointer
moves that change no hover or pressed state skip the layout pass.

Bindings are resolved per tick from providers: `clock` and `cal` (`time.rs`), `opt`, `theme`,
and `app` values the host sets.

## Cards and backdrops (`backdrop.rs`)

Mica and Acrylic only work on top-level windows, so the engine composes the backdrop itself,
once per monitor and kind, cached until the wallpaper, icons or displays change:

1. The wallpaper (`IDesktopWallpaper`, decoded by WIC straight to the target size) laid out
   like Explorer does: fill, fit, stretch, center, tile.
2. The desktop icons, captured with `PrintWindow(PW_RENDERFULLCONTENT)` from the icon list
   view. It renders them over opaque black, so alpha is taken from the brightest channel.
   A WinEvent hook on the list view (out of context, no polling) recaptures them 400 ms after
   icons are added, moved, renamed or selected.
3. Gaussian blur and saturation (D2D effects).

**Frosted** draws the part under the card at 1/4 resolution plus a tint. **Liquid glass**
reads a sharp full-resolution backdrop back to the CPU and renders the card per pixel: the
signed distance to the rounded rect gives the depth into the bezel and the outward normal; a
precomputed table gives the sideways shift of a vertical ray refracted (n = 1.5) by a convex
squircle profile `y = (1 - (1 - x)^4)^(1/4)`; the pixel samples the backdrop that far inward,
with slightly larger shifts for green and blue (dispersion), plus a thin specular line at the
edge, brightest where the edge faces the light or directly away from it. The user picks the
backdrop blur (one cached backdrop per blur level in use). Method after kube.io, "Liquid Glass in the Browser".
This runs only when the card is redrawn (rebuild, drag, icon change). A D2D pixel shader is
the upgrade path if glass ever animates.

Card shadows use the D2D Shadow effect over a command list; the window grows by the shadow
margin and saved positions stay the card's corner.

## Settings (`settings.rs`)

The right-click menu (`menu.rs`) is drawn by the engine: one transparent topmost window over
the work area (not the whole monitor, which the shell would report as a fullscreen app), with
one DComp visual per panel. Panels are squircles with a shadow; rows are separated by hairlines
that start and end at the text inset. It has "Edit" (a submenu of what the package allows) and
"Delete widget". With nothing allowed and no options, right-click shows nothing. A click
outside, Esc or switching apps closes it; arrow keys, Enter and Esc navigate.

Labels follow the Windows display language (`i18n.rs`: en, ru, uk, de, fr, es, it, pt, pl, zh,
ja, ko; English otherwise). `WIDGET_LANG=de` overrides it for testing.
Settings and positions live in `%LOCALAPPDATA%\<exe>\widgets.ini`.

## Tests

`cargo test --release` covers parsing, layout, paints, shadows, timer math, the month grid,
glass refraction, icon keying, snapping and the schema file being current.
