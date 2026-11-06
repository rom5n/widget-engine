//! Widget engine: GPU-rendered desktop widgets for Windows 11.
//!
//! Rendering: D3D11 -> Direct2D + DirectWrite -> DirectComposition. One set of devices per process.
//! Each widget is a window in the desktop layer with a tree of layers ([`Scene`]).
//! Nothing runs between updates: no render loop, no polling.
//!
//! ```no_run
//! use widgine::{Engine, Scene, Widget};
//! struct Hello;
//! impl Widget for Hello {
//!     fn size(&self) -> (f32, f32) { (200.0, 100.0) }
//!     fn build(&mut self, scene: &mut Scene) -> windows::core::Result<()> {
//!         let layer = scene.add_layer(0.0, 0.0, 200.0, 100.0)?;
//!         scene.draw(layer, |_ctx| Ok(()))
//!     }
//! }
//! let mut engine = Engine::new();
//! engine.add("hello", Hello, 100, 100);
//! engine.run().unwrap();
//! ```

mod backdrop;
mod desktop;
mod fonts;
mod gpu;
pub mod host;
mod i18n;
mod layout;
mod markup;
mod menu;
mod package;
mod paint;
mod scene;
mod settings;
pub mod shapes;
mod theme;
pub mod time;

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectComposition::IDCompositionTarget;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

pub use markup::PaintFn;
pub use package::layout_schema;
pub use scene::{LayerId, Scene, Transition};
pub use settings::{Allow, Background, Card, Choice, ChoiceKind, Customization, Font, toggle};
pub use theme::{Theme, rgba};
/// Re-exported so widget code uses the same versions as the engine.
pub use {windows, windows_numerics};

use backdrop::{Backdrops, CardParams};
use gpu::{Gpu, is_device_lost};
use settings::{Action, Resolved, Store, ThemeMode};

/// A widget drawn by the engine.
pub trait Widget {
    /// Size in DIPs.
    fn size(&self) -> (f32, f32);

    /// What end users may change through the right-click menu. Default: nothing.
    fn customization(&self) -> Customization {
        Customization::default()
    }

    /// Creates layers and draws everything. Called at start and after every scene reset:
    /// a setting changed, DPI change, theme or accent change, GPU device loss, Explorer restart.
    fn build(&mut self, scene: &mut Scene) -> Result<()>;

    fn on_event(&mut self, event: Event, scene: &mut Scene) -> Result<()> {
        let _ = (event, scene);
        Ok(())
    }

    /// True where the widget handles clicks itself (DIPs). Everywhere else, double-click and hold
    /// moves the widget.
    fn interactive(&self, x: f32, y: f32) -> bool {
        let _ = (x, y);
        false
    }
}

/// Positions are in DIPs relative to the widget's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// The timer requested with [`Scene::set_timer`] fired.
    Timer,
    /// System time changed: manual change, time zone, DST, or resume from sleep.
    TimeChanged,
    PointerMove { x: f32, y: f32 },
    /// The pointer left the widget.
    PointerLeave,
    /// Left button down. A double-click outside [`Widget::interactive`] parts starts a move instead.
    PointerDown { x: f32, y: f32 },
    PointerUp { x: f32, y: f32 },
    /// Right button released over a panel. Panels get this instead of the settings menu.
    RightClick { x: f32, y: f32 },
    /// The app changed a value with [`set`].
    DataChanged,
    /// Mouse wheel over the widget. `delta` is in notches, positive away from the user (scroll up).
    Wheel { x: f32, y: f32, delta: f32 },
    /// A typed character, for panels with a text field. `'\u{8}'` is Backspace, `'\r'` Enter.
    Char(char),
}

/// Where a widget's window lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// In the desktop layer, below every app window.
    Desktop,
    /// A top-level window above the apps, e.g. a gallery or settings. A drag anywhere that is not
    /// [`Widget::interactive`] moves it. Esc closes it.
    Panel,
    /// Topmost, following the pointer while the user drags a new widget out of a panel. Lands in
    /// the desktop layer where the user lets go.
    Floating,
}

/// Called with (widget id, action) for actions the engine does not handle itself.
pub type ActionFn = Box<dyn FnMut(&str, &str)>;
/// Called with (widget id, x, y) after the user moved a widget. Card corner, screen pixels.
pub type MovedFn = Box<dyn FnMut(&str, i32, i32)>;

#[derive(Default)]
pub struct Engine {
    slots: Vec<Slot>,
    hot_reload: bool,
    keep_alive: bool,
    on_action: Option<ActionFn>,
    on_moved: Option<MovedFn>,
}

struct Slot {
    /// Stable instance id. Key for saved position and settings.
    id: String,
    widget: Box<dyn Widget>,
    /// Package directory and its last stamp, for widgets added with [`Engine::load`].
    package: Option<(std::path::PathBuf, (std::time::SystemTime, usize))>,
    /// `<canvas>` paint callbacks, shared with the package widget.
    canvases: markup::Canvases,
    /// `{app.*}` values, shared with the package widget.
    app: markup::AppValues,
    /// Room around the card for its shadow, in pixels. The window is this much larger on each side.
    margin: i32,
    custom: Customization,
    /// Default top-left corner in screen pixels, used when nothing is saved.
    default_pos: POINT,
    hwnd: Option<HWND>,
    target: Option<IDCompositionTarget>,
    scene: Option<Scene>,
    resolved: Option<Resolved>,
    closed: bool,
    place: Place,
    /// Package previews the widget asked for with [`Scene::add_preview`], alive with its scene.
    previews: Vec<(Scene, Box<dyn Widget>)>,
    /// Store id of the remembered style of this widget's package, see [`Engine::remember_styles`].
    style: Option<String>,
}

fn new_slot(id: &str, widget: Box<dyn Widget>, x: i32, y: i32, place: Place) -> Slot {
    assert!(!id.is_empty() && !id.contains(['.', '=', '\n']), "widget id must be non-empty without '.', '=', newlines");
    let custom = widget.customization();
    Slot {
        id: id.into(),
        widget,
        package: None,
        canvases: Default::default(),
        app: Default::default(),
        margin: 0,
        custom,
        default_pos: POINT { x, y },
        hwnd: None,
        target: None,
        scene: None,
        resolved: None,
        closed: false,
        place,
        previews: Vec::new(),
        style: None,
    }
}

/// Loads a package for a widget window, with [`Engine::plain_cards`] applied.
fn load_package(dir: &std::path::Path) -> Result<package::Package> {
    let mut pkg = package::load(dir)?;
    if PLAIN_CARDS.get() {
        if let Some(card) = &mut pkg.customization.card {
            card.background = Background::Solid;
        }
    }
    Ok(pkg)
}

fn package_slot(id: &str, dir: &std::path::Path, x: i32, y: i32, place: Place) -> Result<Slot> {
    let stamp = package::stamp(dir);
    let pkg = load_package(dir)?;
    let package_id = pkg.id.clone();
    let canvases = markup::Canvases::default();
    let app = markup::AppValues::default();
    let mut slot = new_slot(id, Box::new(markup::Markup::new(pkg, canvases.clone(), app.clone())), x, y, place);
    // Store id of the package's remembered style: no dots, see `Store`.
    slot.style = REMEMBER_STYLES.get().then(|| format!("style-{}", package_id.replace(['.', '='], "-")));
    slot.package = Some((dir.to_path_buf(), stamp));
    slot.canvases = canvases;
    slot.app = app;
    Ok(slot)
}

impl Engine {
    /// Also makes the process per-monitor DPI aware, so screen coordinates the app computes
    /// after this call (for [`Engine::add`]) are real pixels.
    pub fn new() -> Self {
        unsafe {
            // Both fail harmlessly when already set (manifest, host app).
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        Self::default()
    }

    /// Adds a widget. `id` must be stable across runs and unique: it keys the saved position
    /// and settings. (x, y) is the default top-left corner in screen pixels.
    pub fn add(&mut self, id: &str, widget: impl Widget + 'static, x: i32, y: i32) {
        self.slots.push(new_slot(id, Box::new(widget), x, y, Place::Desktop));
    }

    /// Adds a widget from a package directory (`widget.toml`, `layout.xml`, assets).
    /// Fails with a message naming the file and line when the package is invalid.
    pub fn load(&mut self, id: &str, dir: impl AsRef<std::path::Path>, x: i32, y: i32) -> Result<()> {
        self.slots.push(package_slot(id, dir.as_ref(), x, y, Place::Desktop)?);
        Ok(())
    }

    /// Package widgets start with a plain solid card, whatever their `widget.toml` says, like
    /// the previews of [`Scene::add_preview`]. The user can still pick another background.
    /// Call it before loading or opening widgets.
    pub fn plain_cards(&mut self) {
        PLAIN_CARDS.set(true);
    }

    /// The engine remembers the last settings the user picked for each package (background,
    /// size, font, options, ...). A new widget of that package from [`open`] or [`drag_out`]
    /// starts with them. Reset to defaults forgets them.
    pub fn remember_styles(&mut self) {
        REMEMBER_STYLES.set(true);
    }

    /// For apps that live on without widgets, e.g. in the tray: [`Engine::run`] returns only on
    /// `WM_QUIT`, not when the last widget closes, and Esc on a desktop widget does nothing.
    pub fn keep_alive(&mut self) {
        self.keep_alive = true;
    }

    /// Sets `{app.key}` in the package widget `widget_id` before it starts.
    /// While the engine runs, use the free function [`set`].
    pub fn set(&mut self, widget_id: &str, key: &str, value: &str) {
        let slot = self.slots.iter().find(|s| s.id == widget_id).unwrap_or_else(|| panic!("no widget '{widget_id}'"));
        slot.app.borrow_mut().insert(key.into(), value.into());
    }

    /// Receives (widget id, x, y) after the user moved a widget: the card's top-left corner in
    /// screen pixels. The engine saves positions itself; this is for apps that want to know.
    pub fn on_moved(&mut self, f: impl FnMut(&str, i32, i32) + 'static) {
        self.on_moved = Some(Box::new(f));
    }

    /// Paint callback for `<canvas id="canvas_id"/>` in the package widget `widget_id`.
    /// It draws in DIPs with the origin at the canvas's top-left corner, whenever the canvas
    /// is drawn: on every rebuild and when its attributes change.
    pub fn on_paint(
        &mut self,
        widget_id: &str,
        canvas_id: &str,
        paint: impl FnMut(&windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, f32, f32) -> Result<()> + 'static,
    ) {
        let slot = self.slots.iter().find(|s| s.id == widget_id).unwrap_or_else(|| panic!("no widget '{widget_id}'"));
        slot.canvases.borrow_mut().insert(canvas_id.into(), Box::new(paint));
    }

    /// Receives (widget id, action) for `action="..."` clicks and [`Scene::action`] calls that are
    /// not built in (`open:`, `toggle:`, `set:`).
    pub fn on_action(&mut self, f: impl FnMut(&str, &str) + 'static) {
        self.on_action = Some(Box::new(f));
    }

    /// Development mode: reloads a package widget when a file in its directory changes.
    /// An invalid package prints its error to stderr and keeps the last good version on screen.
    pub fn hot_reload(&mut self) {
        self.hot_reload = true;
    }

    /// Shows all widgets and runs the message loop. Returns when all widgets are closed,
    /// or a focused widget gets Esc.
    pub fn run(self) -> Result<()> {
        self.start()?;
        let result = message_loop();
        stop();
        result
    }

    /// Shows all widgets and returns. For apps with their own message loop: the widgets live on
    /// this thread and the app's `DispatchMessageW` drives them. Call [`stop`] before exiting.
    // ponytail: in this mode GPU device loss is noticed on the next draw error, not right away
    // (only `run` waits on the device-removed event).
    pub fn start(self) -> Result<()> {
        unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None)?.into();
            let procs: [(PCWSTR, WNDPROC); 2] = [(WIDGET_CLASS, Some(widget_proc)), (HOST_CLASS, Some(host_proc))];
            for (class, proc) in procs {
                RegisterClassW(&WNDCLASSW {
                    style: CS_DBLCLKS,
                    lpfnWndProc: proc,
                    hInstance: instance,
                    lpszClassName: class,
                    hCursor: LoadCursorW(None, IDC_ARROW)?,
                    ..Default::default()
                });
            }
            // Hidden top-level window. Child windows get no broadcasts (theme, time, power,
            // Explorer restart). Also owns menus and dialogs.
            let host = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                HOST_CLASS,
                w!(""),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            )?;
            TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));
            HOST.with(|h| h.set(Some(host)));
            // Values set before start.
            let _ = PostMessageW(Some(host), WM_APP_SET, WPARAM(0), LPARAM(0));

            let mut state = State {
                gpu: Gpu::new()?,
                host,
                slots: self.slots,
                store: Store::load(),
                backdrops: Backdrops::default(),
                on_action: self.on_action,
                on_moved: self.on_moved,
                fullscreen: None,
                hidden: false,
                keep_alive: self.keep_alive,
            };
            for i in 0..state.slots.len() {
                let r = state.attach(i);
                state.check(r);
            }
            if self.hot_reload {
                SetTimer(Some(host), WATCH_TIMER, WATCH_MS, None);
            }
            STATE.with(|s| *s.borrow_mut() = Some(state));
            watch_icons(host);
            watch_fullscreen(host, true);
        }
        Ok(())
    }
}

/// Closes all widgets started with [`Engine::start`].
pub fn stop() {
    if let Some(host) = with_state(|s| s.host) {
        watch_fullscreen(host, false);
        let _ = unsafe { DestroyWindow(host) };
    }
    STATE.with(|s| s.borrow_mut().take());
}

/// Sets `{app.key}` in the package widget `widget_id` while the engine runs, from any code on
/// the engine's thread, including action and paint callbacks. The widget redraws what changed.
pub fn set(widget_id: &str, key: &str, value: &str) {
    PENDING.with(|p| p.borrow_mut().push((widget_id.into(), key.into(), value.into())));
    if let Some(host) = HOST.with(|h| h.get()) {
        let _ = unsafe { PostMessageW(Some(host), WM_APP_SET, WPARAM(0), LPARAM(0)) };
    }
}

/// Shows a package widget on the desktop while the engine runs. Like [`Engine::load`]: a saved
/// position for `id` wins over (x, y). Opening an id that is already open does nothing.
pub fn open(id: &str, dir: impl AsRef<std::path::Path>, x: i32, y: i32) -> Result<()> {
    command(Command::Open(package_slot(id, dir.as_ref(), x, y, Place::Desktop)?));
    Ok(())
}

/// Shows a widget in a panel: a window above the apps, e.g. a gallery or settings. A drag on
/// any part that is not [`Widget::interactive`] moves it; Esc closes it. (x, y): top-left corner
/// in screen pixels, unless a position is saved for `id`. Opening it again brings it to the front.
pub fn open_panel(id: &str, widget: impl Widget + 'static, x: i32, y: i32) {
    command(Command::Open(new_slot(id, Box::new(widget), x, y, Place::Panel)));
}

/// A new package widget that follows the pointer while the left button is down, and lands on
/// the desktop where the user lets go. Call it from a pointer event, e.g. when a drag starts on
/// a gallery tile.
pub fn drag_out(id: &str, dir: impl AsRef<std::path::Path>) -> Result<()> {
    let dir = dir.as_ref();
    let (w, h) = package::load(dir)?.size;
    let mut c = POINT::default();
    unsafe { GetCursorPos(&mut c)? };
    let s = desktop::scale_at(c);
    let at = POINT { x: c.x - px(w * s / 2.0), y: c.y - px(h * s / 2.0) };
    command(Command::Open(package_slot(id, dir, at.x, at.y, Place::Floating)?));
    command(Command::Drag(id.into()));
    Ok(())
}

/// Decodes an image in memory, e.g. `include_bytes!("logo.png")`, for a widget written in Rust
/// to draw in [`Scene::draw`]. One bitmap pixel is one DIP; scale it with the destination rect.
pub fn load_image(ctx: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, bytes: &[u8]) -> Result<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1> {
    unsafe { backdrop::load_image_bytes(ctx, bytes) }
}

/// Closes a widget or panel. Its saved position and settings stay, for the next time the app
/// opens the same id.
pub fn close(id: &str) {
    command(Command::Close(id.into()));
}

/// Hides or shows all desktop widgets, e.g. from a tray menu. Panels stay.
pub fn set_hidden(hidden: bool) {
    command(Command::Hide(hidden));
}

/// Fullscreen apps hide the widgets on their monitor (games, videos, presentations). On by default.
pub fn set_fullscreen_hiding(on: bool) {
    FULLSCREEN_HIDING.store(on, std::sync::atomic::Ordering::Relaxed);
    if let Some(host) = HOST.with(|h| h.get()) {
        let _ = unsafe { PostMessageW(Some(host), WM_APP_BAR, WPARAM(windows::Win32::UI::Shell::ABN_FULLSCREENAPP as usize), LPARAM(0)) };
    }
}

static FULLSCREEN_HIDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Paint callback for a `<canvas>` of a widget opened with [`open`], like [`Engine::on_paint`].
/// Call it right after `open`.
pub fn on_paint(
    widget_id: &str,
    canvas_id: &str,
    paint: impl FnMut(&windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, f32, f32) -> Result<()> + 'static,
) {
    let find = |slots: &[&Slot]| slots.iter().rev().find(|s| s.id == widget_id).map(|s| s.canvases.clone());
    let queued = COMMANDS.with_borrow(|c| {
        find(&c.iter().filter_map(|c| if let Command::Open(s) = c { Some(s) } else { None }).collect::<Vec<_>>())
    });
    let canvases = queued.or_else(|| with_state(|s| find(&s.slots.iter().collect::<Vec<_>>())).flatten());
    match canvases {
        Some(c) => {
            c.borrow_mut().insert(canvas_id.into(), Box::new(paint));
        }
        None => eprintln!("widget engine: on_paint: no widget '{widget_id}'"),
    }
}

/// An item of [`popup_menu`].
#[derive(Clone, Debug, Default)]
pub struct MenuItem {
    pub label: String,
    /// Returned by [`popup_menu`] when the user picks this item.
    pub action: String,
    /// Checkmark.
    pub checked: bool,
    /// Drawn in red, for destructive items.
    pub danger: bool,
    /// Not empty: a submenu.
    pub items: Vec<MenuItem>,
}

impl MenuItem {
    pub fn new(label: &str, action: &str) -> Self {
        Self { label: label.into(), action: action.into(), ..Default::default() }
    }

    pub fn submenu(label: &str, items: Vec<MenuItem>) -> Self {
        Self { label: label.into(), items, ..Default::default() }
    }

    pub fn checked(self, checked: bool) -> Self {
        Self { checked, ..self }
    }

    pub fn danger(self) -> Self {
        Self { danger: true, ..self }
    }
}

/// The engine's menu (rounded panels, the widgets' right-click menu look) with its top-left
/// corner at (x, y), screen pixels; it flips to stay on screen. Blocks until the user picks an
/// item (its `action`) or dismisses the menu (None). For tray menus: call it from the tray
/// icon's window procedure while the engine runs, not from engine callbacks.
pub fn popup_menu(x: i32, y: i32, items: &[MenuItem]) -> Option<String> {
    fn convert(i: &MenuItem) -> menu::Item {
        let mut item = match i.items.is_empty() {
            true => menu::Item::pick(&i.label, i.checked, Action::App(i.action.clone())),
            false => menu::Item::sub(&i.label, i.items.iter().map(convert).collect()),
        };
        item.danger = i.danger;
        item
    }
    let gfx = with_state(|s| menu::Gfx { d2d: s.gpu.d2d.clone(), dwrite: s.gpu.dwrite.clone(), dcomp: s.gpu.dcomp.clone() })?;
    match menu::run(gfx, POINT { x, y }, items.iter().map(convert).collect())? {
        Action::App(a) => Some(a),
        _ => None,
    }
}

/// `key` (English) in the Windows display language, from the engine's string table. Unknown
/// keys come back unchanged.
pub fn tr(key: &str) -> &str {
    i18n::t(key)
}

/// Locale name for date and time texts in the display language (see [`tr`]); null: the
/// user's own formats. For `GetDateFormatEx` and friends.
pub fn locale() -> PCWSTR {
    i18n::locale()
}

/// What a package is, for galleries.
#[derive(Clone, Debug)]
pub struct PackageInfo {
    pub id: String,
    /// In the display language when the package translates it (`[strings.<lang>]`).
    pub name: String,
    /// `category` from widget.toml, translated the same way. Empty when not set.
    pub category: String,
    /// `variant` from widget.toml (e.g. "Small"), translated. Empty when not set.
    pub variant: String,
    /// DIPs.
    pub size: (f32, f32),
    /// `author`, `version` and `homepage` from widget.toml. Empty when not set.
    pub author: String,
    pub version: String,
    pub homepage: String,
    /// `icon` from widget.toml, as an absolute path.
    pub icon: Option<std::path::PathBuf>,
}

/// Reads and checks a package without showing it. Fails like [`Engine::load`].
pub fn package_info(dir: impl AsRef<std::path::Path>) -> Result<PackageInfo> {
    let p = package::load(dir.as_ref())?;
    Ok(PackageInfo { id: p.id, name: p.name, category: p.category, variant: p.variant, size: p.size, author: p.author, version: p.version, homepage: p.homepage, icon: p.icon })
}

/// Changes to the running engine, applied by the host window outside any state borrow: the
/// public functions above may be called from engine callbacks.
enum Command {
    Open(Slot),
    Close(String),
    /// Starts the system move loop of a floating widget.
    Drag(String),
    /// A floating widget was let go: move it into the desktop layer.
    Dock(String),
    Hide(bool),
}

fn command(c: Command) {
    COMMANDS.with_borrow_mut(|q| q.push(c));
    if let Some(host) = HOST.with(|h| h.get()) {
        let _ = unsafe { PostMessageW(Some(host), WM_APP_SET, WPARAM(0), LPARAM(0)) };
    }
}

/// Applies queued commands, then values from [`set`]. False: the state is busy, try again.
fn flush() -> bool {
    if STATE.with(|s| s.try_borrow_mut().is_err()) {
        return false;
    }
    let mut drags = Vec::new();
    for c in COMMANDS.take() {
        match c {
            Command::Drag(id) => drags.push(id),
            c => {
                with_state(|s| s.command(c));
            }
        }
    }
    with_state(|s| s.apply_pending());
    // The move loop is modal: run it with the state free, so the window's messages get through.
    for id in drags {
        let Some(hwnd) = with_state(|s| s.slots.iter().find(|s| s.id == id && !s.closed).and_then(|s| s.hwnd)).flatten() else { continue };
        unsafe {
            let _ = ReleaseCapture();
            let mut c = POINT::default();
            let _ = GetCursorPos(&mut c);
            let at = LPARAM(((c.y as u16 as isize) << 16) | c.x as u16 as isize);
            let _ = SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(at));
        }
    }
    true
}

const WIDGET_CLASS: PCWSTR = w!("WidgetEngine.Widget");
const HOST_CLASS: PCWSTR = w!("WidgetEngine.Host");
/// Timer ids: widget `i` uses `i + 1`.
const RETRY_TIMER: usize = 0xFFFF_0000;
/// Hot reload check. ponytail: polls file times, use ReadDirectoryChangesW if packages get large.
const WATCH_TIMER: usize = 0xFFFF_0001;
const WATCH_MS: u32 = 300;
/// Debounce for desktop icon changes.
const ICONS_TIMER: usize = 0xFFFF_0002;
const ICONS_MS: u32 = 400;
/// Host messages: apply pending [`set`] values; shell appbar notifications.
const WM_APP_SET: u32 = WM_APP + 1;
const WM_APP_BAR: u32 = WM_APP + 2;

thread_local! {
    static PLAIN_CARDS: Cell<bool> = const { Cell::new(false) };
    static REMEMBER_STYLES: Cell<bool> = const { Cell::new(false) };
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static HOST: Cell<Option<HWND>> = const { Cell::new(None) };
    /// While a widget moves: cursor and window rect when the move started.
    static DRAG: Cell<Option<(POINT, RECT)>> = const { Cell::new(None) };
    /// Values from [`set`], applied by the host window outside any state borrow.
    static PENDING: RefCell<Vec<(String, String, String)>> = const { RefCell::new(Vec::new()) };
    static COMMANDS: RefCell<Vec<Command>> = const { RefCell::new(Vec::new()) };
    /// (icon list view, WinEvent hook, host window), raw handles for the hook callback.
    static ICON_WATCH: Cell<(isize, isize, isize)> = const { Cell::new((0, 0, 0)) };
}

/// Runs `f` on the engine state. None before start or during re-entrant messages.
/// Never hold it across a modal loop (menus, dialogs): timers would be dropped.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

struct State {
    gpu: Gpu,
    host: HWND,
    slots: Vec<Slot>,
    store: Store,
    backdrops: Backdrops,
    on_action: Option<ActionFn>,
    on_moved: Option<MovedFn>,
    /// The monitor (`HMONITOR` value) a fullscreen app runs on: its widgets are hidden.
    fullscreen: Option<isize>,
    /// The app hid the desktop widgets with [`set_hidden`].
    hidden: bool,
    keep_alive: bool,
}

impl State {
    /// Creates the widget window in the desktop layer. Without Explorer, waits for TaskbarCreated.
    fn attach(&mut self, i: usize) -> Result<()> {
        if self.slots[i].closed {
            return Ok(());
        }
        let slot = &mut self.slots[i];
        let saved = self.store.get(&slot.id);
        let coord = |k: &str| saved.get(k).and_then(|v| v.parse().ok());
        let pos = match (coord("x"), coord("y")) {
            (Some(x), Some(y)) if slot.place != Place::Floating => POINT { x, y },
            _ => slot.default_pos,
        };
        let r = settings::resolve(&slot.custom, &saved);
        let (w, h) = settings::size(&slot.custom, &r.options, slot.widget.size());
        let s = desktop::scale_at(pos);
        let (w, h) = (px(w * s), px(h * s));
        let place = slot.place;
        let hwnd = match place {
            Place::Desktop => {
                let Some(icons) = desktop::icon_host() else { return Ok(()) };
                desktop::create_window(WIDGET_CLASS, icons, pos, w, h)?
            }
            _ => desktop::create_popup(WIDGET_CLASS, pos, w, h, place == Place::Floating)?,
        };
        slot.hwnd = Some(hwnd);
        self.retarget(i)?;
        // Popups show once drawn, so they never flash empty.
        unsafe {
            match place {
                Place::Desktop if !self.visible(hwnd) => {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                Place::Desktop => {}
                Place::Panel => {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                    let _ = SetForegroundWindow(hwnd);
                }
                Place::Floating => {
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
            }
        }
        Ok(())
    }

    fn command(&mut self, c: Command) {
        match c {
            Command::Open(slot) => {
                let at = self.slots.iter().position(|s| s.id == slot.id);
                let i = match at {
                    // Already open: a panel comes to the front.
                    Some(i) if !self.slots[i].closed => {
                        if let (Place::Panel, Some(h)) = (self.slots[i].place, self.slots[i].hwnd) {
                            let _ = unsafe { SetForegroundWindow(h) };
                        }
                        return;
                    }
                    // Reuse the closed slot: timer ids are slot indexes.
                    Some(i) => {
                        self.slots[i] = slot;
                        i
                    }
                    None => {
                        self.slots.push(slot);
                        self.slots.len() - 1
                    }
                };
                // A new widget starts with the style last picked for its package.
                let slot = &self.slots[i];
                if let Some(style) = slot.style.clone().filter(|_| !self.store.data.contains_key(&slot.id)) {
                    let id = slot.id.clone();
                    for (k, v) in self.store.get(&style) {
                        self.store.set(&id, &k, &v);
                    }
                }
                let r = self.attach(i);
                self.check(r);
            }
            Command::Close(id) => {
                if let Some(i) = self.slots.iter().position(|s| s.id == id && !s.closed) {
                    self.close(i);
                }
            }
            Command::Dock(id) => {
                let Some(i) = self.slots.iter().position(|s| s.id == id && !s.closed) else { return };
                if self.slots[i].place != Place::Floating {
                    return;
                }
                let hwnd = self.slots[i].hwnd;
                self.forget_window(i);
                if let Some(h) = hwnd {
                    let _ = unsafe { DestroyWindow(h) };
                }
                self.slots[i].place = Place::Desktop;
                let r = self.attach(i);
                self.check(r);
            }
            Command::Hide(on) => {
                self.hidden = on;
                self.show_desktop();
            }
            Command::Drag(_) => unreachable!("flush runs drags"),
        }
    }

    /// Destroys the window. The saved position and settings stay.
    fn close(&mut self, i: usize) {
        let hwnd = self.slots[i].hwnd;
        self.slots[i].closed = true;
        self.forget_window(i);
        if let Some(h) = hwnd {
            let _ = unsafe { DestroyWindow(h) };
        }
        if !self.keep_alive && self.slots.iter().all(|s| s.closed) {
            unsafe { PostQuitMessage(0) };
        }
    }

    /// Desktop widgets are visible unless the app hid them or a fullscreen app runs on their monitor.
    fn show_desktop(&mut self) {
        for slot in self.slots.iter().filter(|s| s.place == Place::Desktop) {
            if let Some(h) = slot.hwnd {
                let _ = unsafe { ShowWindow(h, if self.visible(h) { SW_SHOWNA } else { SW_HIDE }) };
            }
        }
    }

    fn visible(&self, hwnd: HWND) -> bool {
        use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
        !self.hidden && self.fullscreen.is_none_or(|m| m != unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) }.0 as isize)
    }

    /// New composition target for the window, then a full rebuild.
    fn retarget(&mut self, i: usize) -> Result<()> {
        let slot = &mut self.slots[i];
        let Some(hwnd) = slot.hwnd else { return Ok(()) };
        slot.target = Some(unsafe { self.gpu.dcomp.CreateTargetForHwnd(hwnd, true)? });
        self.rebuild(i)
    }

    /// Fresh scene with the current settings, DPI, and theme. The widget draws everything.
    fn rebuild(&mut self, i: usize) -> Result<()> {
        let slot = &mut self.slots[i];
        let (Some(hwnd), Some(target)) = (slot.hwnd, &slot.target) else { return Ok(()) };
        let r = settings::resolve(&slot.custom, &self.store.get(&slot.id));
        let scale = desktop::scale_of(hwnd) * r.size;
        let theme = theme_for(&r);
        let (w, h) = settings::size(&slot.custom, &r.options, slot.widget.size());
        let card_pos = desktop::screen_pos(hwnd);
        let card_pos = POINT { x: card_pos.x + slot.margin, y: card_pos.y + slot.margin };
        let m = if r.card.is_some_and(|c| c.shadow) { paint::shadow_margin(&backdrop::card_shadows(r.card.unwrap().background)) } else { 0.0 };
        // Whole pixels: the glass bitmap is drawn pixel for pixel, and a half-pixel offset
        // resamples it into a torn-looking rim.
        let m = (m * scale).round() / scale;
        slot.margin = px(m * scale);
        desktop::place(hwnd, card_pos, px(w * scale), px(h * scale), slot.margin)?;

        let mut scene = Scene::new(&self.gpu, scale, (w, h), theme, r.font.clone(), r.options.clone())?;
        unsafe {
            target.SetRoot(&scene.root)?;
            scene.root.SetOffsetX2(slot.margin as f32)?;
            scene.root.SetOffsetY2(slot.margin as f32)?;
        }
        if r.card.is_some() {
            scene.card = Some(scene.add_layer(-m, -m, w + 2.0 * m, h + 2.0 * m)?);
        }
        slot.resolved = Some(r);
        slot.widget.build(&mut scene)?;
        slot.previews.clear();
        let pos = desktop::screen_pos(hwnd);
        let origin = POINT { x: pos.x + slot.margin, y: pos.y + slot.margin };
        for p in std::mem::take(&mut scene.previews) {
            match build_preview(&self.gpu, &scene, origin, &p) {
                Ok(v) => slot.previews.push(v),
                Err(e) => eprintln!("widget engine: preview of {}: {e}", p.dir.display()),
            }
        }
        slot.scene = Some(scene);
        self.draw_card(i)?;
        self.finish(i)
    }

    /// Redraws the engine-drawn card. Frosted cards depend on the widget's screen position.
    fn draw_card(&mut self, i: usize) -> Result<()> {
        let slot = &mut self.slots[i];
        let (Some(hwnd), Some(scene), Some(r)) = (slot.hwnd, slot.scene.as_mut(), slot.resolved.as_ref()) else {
            return Ok(());
        };
        let (Some(card), Some(layer)) = (r.card, scene.card) else { return Ok(()) };
        let rc = desktop::window_rect(hwnd);
        let center = POINT { x: (rc.left + rc.right) / 2, y: (rc.top + rc.bottom) / 2 };
        let frost = match card.background {
            Background::Frosted => self.backdrops.frost(&self.gpu.ctx, center),
            _ => None,
        };
        let glass = match card.background {
            Background::Glass => self.backdrops.glass(&self.gpu.ctx, center, r.blur * scene.scale()),
            _ => None,
        };
        let params = CardParams {
            card,
            theme: *scene.theme(),
            opacity: r.opacity,
            tint: r.tint,
            veil: r.veil,
            origin: POINT { x: rc.left + slot.margin, y: rc.top + slot.margin },
            scale: scene.scale(),
            size: settings::size(&slot.custom, &r.options, slot.widget.size()),
            frost,
            glass,
        };
        let d2d = scene.d2d().clone();
        let shadows = if card.shadow { backdrop::card_shadows(card.background) } else { Vec::new() };
        let m = (paint::shadow_margin(&shadows) * scene.scale()).round() / scene.scale();
        scene.draw(layer, |ctx| unsafe {
            let mut base = windows_numerics::Matrix3x2::default();
            ctx.GetTransform(&mut base);
            ctx.SetTransform(&(windows_numerics::Matrix3x2::translation(m, m) * base));
            paint::with_shadows(ctx, &shadows, || backdrop::draw_card(ctx, &d2d, &params))
        })
    }

    fn rebuild_all(&mut self) {
        for i in 0..self.slots.len() {
            let r = self.rebuild(i);
            self.check(r);
        }
    }

    fn event(&mut self, i: usize, event: Event) -> Result<()> {
        let Some(slot) = self.slots.get_mut(i) else { return Ok(()) };
        let Some(scene) = slot.scene.as_mut() else { return Ok(()) };
        slot.widget.on_event(event, scene)?;
        let actions = std::mem::take(&mut scene.actions);
        let mut rebuild = scene.rebuild_requested;
        for a in actions {
            rebuild |= self.run_action(i, &a);
        }
        if rebuild {
            return self.rebuild(i);
        }
        self.finish(i)
    }

    /// Built-in actions, then the app callback. True when a setting changed (rebuild).
    fn run_action(&mut self, i: usize, action: &str) -> bool {
        let id = self.slots[i].id.clone();
        if let Some(target) = action.strip_prefix("open:") {
            // The package ships with the app, so it may open what the app could open itself.
            unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(target.trim()), None, None, SW_SHOWNORMAL) };
            return false;
        }
        let set = action.strip_prefix("set:").and_then(|r| r.split_once('='));
        let key = action.strip_prefix("toggle:").or(set.map(|(k, _)| k)).map(str::trim);
        if let Some(key) = key {
            let Some(choice) = self.slots[i].custom.options.iter().find(|o| o.key == key) else {
                eprintln!("widget engine: action '{action}': no option '{key}'");
                return false;
            };
            let current = self.slots[i].resolved.as_ref().and_then(|r| r.options.get(key).cloned()).unwrap_or_default();
            let value = match set {
                Some((_, v)) => v.trim().to_string(),
                None => {
                    let at = choice.values.iter().position(|(v, _)| *v == current).map_or(0, |p| p + 1);
                    choice.values[at % choice.values.len()].0.clone()
                }
            };
            if !choice.values.iter().any(|(v, _)| *v == value) {
                eprintln!("widget engine: action '{action}': '{value}' is not a value of '{key}'");
                return false;
            }
            self.set_setting(i, &format!("opt.{key}"), &value);
            return true;
        }
        match &mut self.on_action {
            // The app may have changed what its canvases draw: redraw the widget.
            Some(f) => {
                f(&id, action);
                true
            }
            None => {
                eprintln!("widget engine: action '{action}' from '{id}': no on_action handler");
                false
            }
        }
    }

    /// Pointer input for the widget at `hwnd`, `client` in window pixels.
    fn pointer(&mut self, hwnd: HWND, client: POINT, make: impl FnOnce(f32, f32) -> Event) {
        let Some(i) = self.slot_of(hwnd) else { return };
        let Some((x, y)) = self.to_dips(i, client) else { return };
        let r = self.event(i, make(x, y));
        self.check(r);
    }

    fn to_dips(&self, i: usize, client: POINT) -> Option<(f32, f32)> {
        let slot = &self.slots[i];
        let s = slot.scene.as_ref()?.scale();
        Some(((client.x - slot.margin) as f32 / s, (client.y - slot.margin) as f32 / s))
    }

    /// True over parts the widget handles clicks on itself (`client` in window pixels).
    fn interactive(&self, hwnd: HWND, client: POINT) -> bool {
        let Some(i) = self.slot_of(hwnd) else { return false };
        self.to_dips(i, client).is_some_and(|(x, y)| self.slots[i].widget.interactive(x, y))
    }

    fn margin_of(&self, hwnd: HWND) -> i32 {
        self.slot_of(hwnd).map_or(0, |i| self.slots[i].margin)
    }

    fn event_all(&mut self, event: Event) {
        for i in 0..self.slots.len() {
            let r = self.event(i, event);
            self.check(r);
        }
    }

    /// Applies the requested timer, then commits all changes of this update at once.
    fn finish(&mut self, i: usize) -> Result<()> {
        if let Some(ms) = self.slots[i].scene.as_mut().and_then(|s| s.timer.take()) {
            unsafe { SetTimer(Some(self.host), i + 1, ms, None) };
        }
        unsafe { self.gpu.dcomp.Commit() }
    }

    /// GPU reset or driver update: recreate every device, target, and scene.
    fn recover_device(&mut self) {
        let gpu = match Gpu::new() {
            Ok(gpu) => gpu,
            Err(_) => {
                // GPU still resetting. Try again shortly.
                unsafe { SetTimer(Some(self.host), RETRY_TIMER, 1000, None) };
                return;
            }
        };
        self.backdrops.clear();
        for slot in &mut self.slots {
            slot.scene = None;
            slot.target = None;
        }
        self.gpu = gpu;
        for i in 0..self.slots.len() {
            if let Err(e) = self.retarget(i) {
                eprintln!("widget engine: rebuild after device loss failed: {e}");
            }
        }
    }

    /// Hot reload: loads changed packages again. A broken package keeps the old widget.
    fn reload_changed(&mut self) {
        for i in 0..self.slots.len() {
            let slot = &mut self.slots[i];
            let Some((dir, stamp)) = &mut slot.package else { continue };
            let now = package::stamp(dir);
            if now == *stamp {
                continue;
            }
            *stamp = now;
            match load_package(dir) {
                Ok(pkg) => {
                    eprintln!("widget engine: reloaded {}", dir.display());
                    slot.custom = pkg.customization.clone();
                    slot.widget = Box::new(markup::Markup::new(pkg, slot.canvases.clone(), slot.app.clone()));
                    let r = self.rebuild(i);
                    self.check(r);
                }
                Err(e) => eprintln!("widget engine: {e}"),
            }
        }
    }

    /// Widget windows die with Explorer. Recreate them when the desktop is back.
    fn reattach_all(&mut self) {
        watch_icons(self.host);
        for i in 0..self.slots.len() {
            let alive = self.slots[i].hwnd.is_some_and(|h| unsafe { IsWindow(Some(h)) }.as_bool());
            if !alive {
                self.forget_window(i);
                let r = self.attach(i);
                self.check(r);
            }
        }
    }

    fn forget_window(&mut self, i: usize) {
        let slot = &mut self.slots[i];
        slot.hwnd = None;
        slot.target = None;
        slot.previews.clear();
        slot.scene = None;
    }

    fn slot_of(&self, hwnd: HWND) -> Option<usize> {
        self.slots.iter().position(|s| s.hwnd == Some(hwnd))
    }

    /// During a drag: frosted and glass cards follow the backdrop under them.
    fn moving(&mut self, hwnd: HWND) {
        let Some(i) = self.slot_of(hwnd) else { return };
        self.redraw_backdrop(i);
    }

    fn redraw_backdrop(&mut self, i: usize) {
        let uses = self.slots[i].resolved.as_ref().and_then(|r| r.card).is_some_and(|c| c.background.uses_wallpaper());
        if uses {
            let r = self.draw_card(i).and_then(|_| unsafe { self.gpu.dcomp.Commit() });
            self.check(r);
        }
    }

    /// Applies values from [`set`] and redraws the widgets they touch.
    fn apply_pending(&mut self) {
        let pending = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
        let mut touched = Vec::new();
        for (id, key, value) in pending {
            let Some(i) = self.slots.iter().position(|s| s.id == id) else {
                eprintln!("widget engine: set: no widget '{id}'");
                continue;
            };
            self.slots[i].app.borrow_mut().insert(key, value);
            if !touched.contains(&i) {
                touched.push(i);
            }
        }
        for i in touched {
            let r = self.event(i, Event::DataChanged);
            self.check(r);
        }
    }

    /// A fullscreen app (game, video, presentation) started or ended on `monitor`: hide or show
    /// the widgets there.
    fn set_fullscreen(&mut self, monitor: Option<isize>) {
        if self.fullscreen == monitor {
            return;
        }
        self.fullscreen = monitor;
        self.show_desktop();
        if monitor.is_none() {
            // Time moved on while hidden.
            self.event_all(Event::TimeChanged);
        }
    }

    /// While dragging: the card edges snap to the work area edges and to other widgets' cards
    /// when they come within a few pixels. `card` is in screen pixels.
    fn snap(&self, hwnd: HWND, card: RECT) -> RECT {
        let reach = (SNAP_DIPS * desktop::scale_of(hwnd)).round() as i32;
        let area = desktop::work_area(card);
        let mut xs = vec![(area.left, false), (area.right, true)];
        let mut ys = vec![(area.top, false), (area.bottom, true)];
        let gap = (SNAP_GAP_DIPS * desktop::scale_of(hwnd)).round() as i32;
        for s in self.slots.iter().filter(|s| s.hwnd.is_some_and(|h| h != hwnd)) {
            let r = desktop::window_rect(s.hwnd.unwrap());
            let o = RECT { left: r.left + s.margin, top: r.top + s.margin, right: r.right - s.margin, bottom: r.bottom - s.margin };
            // Side by side with a gap, or aligned edges.
            xs.extend([(o.left - gap, true), (o.right + gap, false), (o.left, false), (o.right, true)]);
            ys.extend([(o.top - gap, true), (o.bottom + gap, false), (o.top, false), (o.bottom, true)]);
        }
        let (dx, dy) = (snap_axis(card.left, card.right, &xs, reach), snap_axis(card.top, card.bottom, &ys, reach));
        RECT { left: card.left + dx, top: card.top + dy, right: card.right + dx, bottom: card.bottom + dy }
    }

    /// Desktop icons changed: compose the backdrops again.
    fn icons_changed(&mut self) {
        self.backdrops.clear();
        for i in 0..self.slots.len() {
            self.redraw_backdrop(i);
        }
    }

    /// End of a drag: save the position, rebuild if the monitor DPI differs.
    fn moved(&mut self, hwnd: HWND) {
        let Some(i) = self.slot_of(hwnd) else { return };
        // Saved positions are the card's corner, without the shadow margin.
        let m = self.slots[i].margin;
        let pos = desktop::screen_pos(hwnd);
        let pos = POINT { x: pos.x + m, y: pos.y + m };
        let id = self.slots[i].id.clone();
        self.store.set(&id, "x", &pos.x.to_string());
        self.store.set(&id, "y", &pos.y.to_string());
        if let Some(f) = &mut self.on_moved {
            f(&id, pos.x, pos.y);
        }
        if self.slots[i].place == Place::Floating {
            // Not from inside this window's own message: the dock destroys it.
            command(Command::Dock(id));
            return;
        }
        let size = self.slots[i].resolved.as_ref().map_or(1.0, |r| r.size);
        let old = self.slots[i].scene.as_ref().map(|s| s.scale());
        let r = if old != Some(desktop::scale_of(hwnd) * size) {
            self.rebuild(i)
        } else {
            self.draw_card(i).and_then(|_| unsafe { self.gpu.dcomp.Commit() })
        };
        self.check(r);
    }

    /// Applies a menu or dialog result.
    /// A setting the user picked, also remembered for new widgets of the same package.
    fn set_setting(&mut self, i: usize, key: &str, value: &str) {
        let id = self.slots[i].id.clone();
        self.store.set(&id, key, value);
        if let Some(style) = self.slots[i].style.clone() {
            self.store.set(&style, key, value);
        }
    }

    fn apply(&mut self, i: usize, action: Action) {
        let id = self.slots[i].id.clone();
        match action {
            Action::Set(k, v) => self.set_setting(i, &k, &v),
            Action::Reset => {
                self.store.reset(&id);
                if let Some(style) = &self.slots[i].style {
                    self.store.remove(style);
                }
            }
            Action::Delete => {
                self.store.remove(&id);
                // Tell the app, so it stops loading this widget.
                if let Some(f) = &mut self.on_action {
                    f(&id, "delete");
                }
                self.close(i);
                return;
            }
            // Dialogs run outside the state borrow, see `context_menu`.
            Action::PickFont | Action::PickColor(_) | Action::PickPath(..) => return,
            // Only from `popup_menu`.
            Action::App(_) => return,
        }
        let r = self.rebuild(i);
        self.check(r);
    }

    fn check(&mut self, r: Result<()>) {
        match r {
            Err(e) if is_device_lost(&e) => self.recover_device(),
            Err(e) => eprintln!("widget engine: {e}"),
            Ok(()) => {}
        }
    }
}

/// The system theme with the user's theme, accent, and text color picks.
fn theme_for(r: &Resolved) -> Theme {
    let mut theme = Theme::current();
    match r.theme {
        ThemeMode::Light => theme.dark = false,
        ThemeMode::Dark => theme.dark = true,
        ThemeMode::System => {}
    }
    // On a background color the text goes dark or light to stay readable.
    if let (Some(t), ThemeMode::System) = (r.tint, r.theme) {
        let (cr, cg, cb) = ((t >> 16) & 0xFF, (t >> 8) & 0xFF, t & 0xFF);
        theme.dark = 0.299 * cr as f32 + 0.587 * cg as f32 + (0.114 * cb as f32) < 150.0;
    }
    if let Some(c) = r.accent {
        theme.accent = theme::rgb(c);
    }
    theme.text = r.text.map(theme::rgb);
    theme
}

/// A package drawn small inside another widget's scene, with default settings and `p.values`
/// as its `{app.*}` values. Static: it gets no timers or input. `origin`: the parent card's
/// top-left corner in screen pixels, for wallpaper-based cards.
fn build_preview(
    gpu: &Gpu,
    parent: &Scene,
    origin: POINT,
    p: &scene::Preview,
) -> Result<(Scene, Box<dyn Widget>)> {
    let pkg = package::load(&p.dir)?;
    let (pw, ph) = pkg.size;
    let custom = pkg.customization.clone();
    let app = markup::AppValues::default();
    app.borrow_mut().extend(p.values.iter().cloned());
    let mut widget = markup::Markup::new(pkg, Default::default(), app);
    let r = settings::resolve(&custom, &Default::default());
    let ps = parent.scale();
    let k = (p.w / pw).min(p.h / ph).min(1.0);
    let s = ps * k;
    let mut sub = Scene::new(gpu, s, (pw, ph), theme_for(&r), r.font.clone(), r.options.clone())?;
    // Centered in its box, on whole pixels.
    let x = ((p.x + (p.w - pw * k) / 2.0) * ps).round();
    let y = ((p.y + (p.h - ph * k) / 2.0) * ps).round();
    unsafe {
        sub.root.SetOffsetX2(x)?;
        sub.root.SetOffsetY2(y)?;
        parent.root.AddVisual(&sub.root, false, None)?;
    }
    // Previews sit on the gallery, not the wallpaper: every card shows as a plain solid one.
    if let Some(card) = r.card {
        let layer = sub.add_layer(0.0, 0.0, pw, ph)?;
        let params = CardParams {
            card: Card { background: Background::Solid, ..card },
            theme: *sub.theme(),
            opacity: 1.0,
            tint: r.tint,
            veil: r.veil,
            origin: POINT { x: origin.x + x as i32, y: origin.y + y as i32 },
            scale: s,
            size: (pw, ph),
            frost: None,
            glass: None,
        };
        let d2d = sub.d2d().clone();
        sub.draw(layer, |ctx| unsafe { backdrop::draw_card(ctx, &d2d, &params) })?;
    }
    widget.build(&mut sub)?;
    Ok((sub, Box::new(widget)))
}

/// Right-click menu. Menus and dialogs run modal loops, so the state is borrowed only
/// before and after them, never during.
fn context_menu(hwnd: HWND, pt: POINT) {
    let Some((i, host, resolved, gfx, items)) = with_state(|s| {
        let i = s.slot_of(hwnd)?;
        let slot = &s.slots[i];
        let resolved = slot.resolved.clone()?;
        let items = settings::menu_items(&slot.custom, &resolved)?;
        let g = &s.gpu;
        let gfx = menu::Gfx { d2d: g.d2d.clone(), dwrite: g.dwrite.clone(), dcomp: g.dcomp.clone() };
        Some((i, s.host, resolved, gfx, items))
    })
    .flatten() else {
        return;
    };
    let Some(action) = menu::run(gfx, pt, items) else { return };
    let action = match action {
        Action::PickFont => match settings::pick_font(host, &resolved.font) {
            Some((family, weight)) => {
                with_state(|s| s.set_setting(i, "weight", &weight.to_string()));
                Action::Set("font".into(), family)
            }
            None => return,
        },
        Action::PickColor(key) => {
            let current = match key.as_str() {
                "tint" => resolved.tint,
                "accent" => resolved.accent,
                _ => resolved.text,
            };
            match settings::pick_color(host, current) {
                Some(rgb) => Action::Set(key, format!("{rgb:06X}")),
                None => return,
            }
        }
        Action::PickPath(key, kind) => match settings::pick_path(host, kind) {
            Some(path) => Action::Set(key, path),
            None => return,
        },
        a => a,
    };
    with_state(|s| s.apply(i, action));
}

fn message_loop() -> Result<()> {
    let mut msg = MSG::default();
    loop {
        // Sleep until a message arrives or the GPU signals device removal.
        let lost = with_state(|s| s.gpu.lost_event).filter(|h| !h.is_invalid());
        let handles = lost.map(|h| [h]);
        let r = unsafe {
            MsgWaitForMultipleObjectsEx(handles.as_ref().map(|h| &h[..]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
        if lost.is_some() && r == WAIT_OBJECT_0 {
            with_state(|s| s.recover_device());
            continue;
        }
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                return Ok(());
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

extern "system" fn widget_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_LBUTTONDOWN => {
                let _ = SetFocus(Some(hwnd));
                SetCapture(hwnd);
                with_state(|s| s.pointer(hwnd, point_from(lparam), |x, y| Event::PointerDown { x, y }));
                return LRESULT(0);
            }
            // Double-click and hold moves the widget: hand the held button to the system move loop.
            // On clickable parts the second click stays a click.
            WM_LBUTTONDBLCLK => {
                let p = point_from(lparam);
                if with_state(|s| s.interactive(hwnd, p)).unwrap_or(false) {
                    SetCapture(hwnd);
                    with_state(|s| s.pointer(hwnd, p, |x, y| Event::PointerDown { x, y }));
                } else {
                    let _ = ReleaseCapture();
                    let mut c = POINT::default();
                    let _ = GetCursorPos(&mut c);
                    let at = LPARAM(((c.y as u16 as isize) << 16) | c.x as u16 as isize);
                    let _ = SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(at));
                }
                return LRESULT(0);
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                with_state(|s| s.pointer(hwnd, point_from(lparam), |x, y| Event::PointerUp { x, y }));
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                let p = point_from(lparam);
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut track);
                with_state(|s| s.pointer(hwnd, p, |x, y| Event::PointerMove { x, y }));
            }
            WM_MOUSELEAVE => {
                with_state(|s| {
                    if let Some(i) = s.slot_of(hwnd) {
                        let r = s.event(i, Event::PointerLeave);
                        s.check(r);
                    }
                });
            }
            // Keep the card inside its monitor's work area while dragging. The shadow may overhang.
            WM_ENTERSIZEMOVE => {
                let mut c = POINT::default();
                let _ = GetCursorPos(&mut c);
                DRAG.set(Some((c, desktop::window_rect(hwnd))));
            }
            WM_MOVING => {
                let r = &mut *(lparam.0 as *mut RECT);
                // The move loop adds each mouse step to the rect we returned last time, so a
                // snapped edge would never let go of a slow drag. Start from where the pointer
                // says the window should be instead.
                if let Some((start, from)) = DRAG.get() {
                    let mut c = POINT::default();
                    let _ = GetCursorPos(&mut c);
                    let (dx, dy) = (c.x - start.x, c.y - start.y);
                    *r = RECT { left: from.left + dx, top: from.top + dy, right: from.right + dx, bottom: from.bottom + dy };
                }
                let m = with_state(|s| s.margin_of(hwnd)).unwrap_or(0);
                let inner = RECT { left: r.left + m, top: r.top + m, right: r.right - m, bottom: r.bottom - m };
                let card = with_state(|s| s.snap(hwnd, inner)).unwrap_or(inner);
                let card = desktop::keep_on_screen(card);
                *r = RECT { left: card.left - m, top: card.top - m, right: card.right + m, bottom: card.bottom + m };
                return LRESULT(1);
            }
            WM_MOVE => {
                with_state(|s| s.moving(hwnd));
            }
            WM_EXITSIZEMOVE => {
                DRAG.set(None);
                with_state(|s| s.moved(hwnd));
            }
            // Right-click: settings, when the developer allows any.
            WM_RBUTTONDOWN => return LRESULT(0),
            WM_RBUTTONUP if place_of(hwnd) == Some(Place::Panel) => {
                with_state(|s| s.pointer(hwnd, point_from(lparam), |x, y| Event::RightClick { x, y }));
                return LRESULT(0);
            }
            WM_RBUTTONUP => {
                let mut pt = point_from(lparam);
                let _ = ClientToScreen(hwnd, &mut pt);
                context_menu(hwnd, pt);
                return LRESULT(0);
            }
            // Keyboard: Shift+F10 or the menu key.
            WM_CONTEXTMENU => {
                let p = point_from(lparam);
                let pt = if p.x == -1 && p.y == -1 { desktop::screen_pos(hwnd) } else { p };
                context_menu(hwnd, pt);
                return LRESULT(0);
            }
            // Panels: a drag on any part the widget does not handle moves the window.
            WM_NCHITTEST if place_of(hwnd) == Some(Place::Panel) => {
                let mut p = point_from(lparam);
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut p);
                let mine = with_state(|s| s.interactive(hwnd, p)).unwrap_or(true);
                return LRESULT(if mine { HTCLIENT } else { HTCAPTION } as isize);
            }
            // No maximize on double-click, no system menu on right-click.
            WM_NCLBUTTONDBLCLK | WM_NCRBUTTONDOWN | WM_NCRBUTTONUP if place_of(hwnd) == Some(Place::Panel) => {
                return LRESULT(0);
            }
            WM_CHAR => {
                let c = char::from_u32(wparam.0 as u32).filter(|c| !c.is_control() || matches!(c, '\u{8}' | '\r'));
                if let Some(c) = c {
                    with_state(|s| {
                        if let Some(i) = s.slot_of(hwnd) {
                            let r = s.event(i, Event::Char(c));
                            s.check(r);
                        }
                    });
                }
                return LRESULT(0);
            }
            WM_MOUSEWHEEL => {
                let mut p = point_from(lparam);
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut p);
                let delta = ((wparam.0 >> 16) as u16 as i16) as f32 / WHEEL_DELTA as f32;
                with_state(|s| s.pointer(hwnd, p, |x, y| Event::Wheel { x, y, delta }));
                return LRESULT(0);
            }
            // Esc closes a panel. Without keep_alive it quits the app, as before tray apps existed.
            WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
                match place_of(hwnd) {
                    Some(Place::Panel) => {
                        if let Some(id) = with_state(|s| s.slot_of(hwnd).map(|i| s.slots[i].id.clone())).flatten() {
                            close(&id);
                        }
                    }
                    _ if with_state(|s| s.keep_alive).unwrap_or(true) => {}
                    _ => PostQuitMessage(0),
                }
                return LRESULT(0);
            }
            WM_DESTROY => {
                with_state(|s| {
                    if let Some(i) = s.slot_of(hwnd) {
                        s.forget_window(i);
                    }
                });
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

extern "system" fn host_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                // Engine timers are one-shot.
                let _ = KillTimer(Some(hwnd), wparam.0);
                let handled = with_state(|s| match wparam.0 {
                    RETRY_TIMER => s.recover_device(),
                    WATCH_TIMER => {
                        s.reload_changed();
                        SetTimer(Some(hwnd), WATCH_TIMER, WATCH_MS, None);
                    }
                    ICONS_TIMER => s.icons_changed(),
                    id => {
                        let r = s.event(id - 1, Event::Timer);
                        s.check(r);
                    }
                });
                if handled.is_none() {
                    // State busy (re-entrant message). Retry instead of dropping the tick.
                    SetTimer(Some(hwnd), wparam.0, 50, None);
                }
                return LRESULT(0);
            }
            WM_APP_SET => {
                if !flush() {
                    // State busy: try again after the current message.
                    let _ = PostMessageW(Some(hwnd), WM_APP_SET, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
            // The shell's notice is only a hint: after some games it says "fullscreen" for the
            // desktop itself, and it can miss the end. Look at the window in front instead.
            WM_APP_BAR if wparam.0 == windows::Win32::UI::Shell::ABN_FULLSCREENAPP as usize => {
                with_state(|s| s.set_fullscreen(fullscreen_in_front()));
                return LRESULT(0);
            }
            WM_TIMECHANGE => {
                with_state(|s| s.event_all(Event::TimeChanged));
            }
            WM_POWERBROADCAST => {
                if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize {
                    with_state(|s| s.event_all(Event::TimeChanged));
                }
                return LRESULT(1);
            }
            WM_SETTINGCHANGE if wparam.0 == SPI_SETDESKWALLPAPER.0 as usize => {
                with_state(|s| {
                    s.backdrops.clear();
                    s.rebuild_all();
                });
            }
            WM_SETTINGCHANGE if lparam_is(lparam, "ImmersiveColorSet") => {
                with_state(|s| s.rebuild_all());
            }
            WM_DWMCOLORIZATIONCOLORCHANGED | WM_DISPLAYCHANGE => {
                with_state(|s| {
                    s.backdrops.clear();
                    s.rebuild_all();
                });
            }
            m if m != 0 && m == TASKBAR_CREATED.get() => {
                with_state(|s| s.reattach_all());
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// Cards show the desktop icons behind them. Explorer reports icon changes (added, removed,
/// moved, renamed, selected) as WinEvents on the icon list view; those restart a short timer
/// that captures the icons again. No polling.
fn watch_icons(host: HWND) {
    use windows::Win32::UI::Accessibility::*;
    let (_, old, _) = ICON_WATCH.get();
    if old != 0 {
        let _ = unsafe { UnhookWinEvent(HWINEVENTHOOK(old as _)) };
    }
    ICON_WATCH.set((0, 0, 0));
    let Some(list) = desktop::icon_list() else { return };
    let mut pid = 0;
    let tid = unsafe { GetWindowThreadProcessId(list, Some(&mut pid)) };
    let hook = unsafe {
        SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_NAMECHANGE, None, Some(icon_event), pid, tid, WINEVENT_OUTOFCONTEXT)
    };
    ICON_WATCH.set((list.0 as isize, hook.0 as isize, host.0 as isize));
}

unsafe extern "system" fn icon_event(
    _: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let (list, _, host) = ICON_WATCH.get();
    if list != 0 && hwnd.0 as isize == list {
        unsafe { SetTimer(Some(HWND(host as _)), ICONS_TIMER, ICONS_MS, None) };
    }
}

/// Snap distance and the gap kept between snapped widgets, in DIPs.
const SNAP_DIPS: f32 = 12.0;
const SNAP_GAP_DIPS: f32 = 12.0;

/// Shift that moves the nearer edge of `lo..hi` onto a target within `reach`, or 0.
/// Each target is (position, true = the high edge snaps to it).
fn snap_axis(lo: i32, hi: i32, targets: &[(i32, bool)], reach: i32) -> i32 {
    targets
        .iter()
        .map(|&(t, high)| t - if high { hi } else { lo })
        .filter(|d| d.abs() <= reach)
        .min_by_key(|d| d.abs())
        .unwrap_or(0)
}

/// Registers the host as a shell appbar, only to be told about fullscreen apps
/// (`ABN_FULLSCREENAPP`). It reserves no screen space. Each change of the window in front
/// also asks for a fresh look.
fn watch_fullscreen(host: HWND, on: bool) {
    use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
    use windows::Win32::UI::Shell::*;
    thread_local! {
        static HOOK: std::cell::Cell<(isize, isize)> = const { std::cell::Cell::new((0, 0)) };
    }
    /// Posted, not handled here: the state may be busy when the hook runs.
    unsafe extern "system" fn moved(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
        let host = HWND(HOOK.get().1 as _);
        let _ = unsafe { PostMessageW(Some(host), WM_APP_BAR, WPARAM(ABN_FULLSCREENAPP as usize), LPARAM(0)) };
    }
    let mut bar = APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, hWnd: host, uCallbackMessage: WM_APP_BAR, ..Default::default() };
    unsafe {
        SHAppBarMessage(if on { ABM_NEW } else { ABM_REMOVE }, &mut bar);
        let (old, _) = HOOK.get();
        if old != 0 {
            let _ = UnhookWinEvent(HWINEVENTHOOK(old as _));
        }
        let hook = if on { SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(moved), 0, 0, WINEVENT_OUTOFCONTEXT).0 as isize } else { 0 };
        HOOK.set((hook, host.0 as isize));
    }
}

/// The monitor of the window in front, if that is a fullscreen app: it covers its whole monitor,
/// taskbar included, and is not the desktop, the taskbar or one of ours (menus, dialogs).
fn fullscreen_in_front() -> Option<isize> {
    if !FULLSCREEN_HIDING.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONULL, MONITORINFO, MonitorFromWindow};
    unsafe {
        let w = GetForegroundWindow();
        if w.is_invalid() {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(w, Some(&mut pid));
        if pid == windows::Win32::System::Threading::GetCurrentProcessId() {
            return None;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(w, &mut class) as usize;
        if is_shell(&String::from_utf16_lossy(&class[..n])) {
            return None;
        }
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mut r = RECT::default();
        let monitor = MonitorFromWindow(w, MONITOR_DEFAULTTONULL);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() || GetWindowRect(w, &mut r).is_err() {
            return None;
        }
        covers(r, info.rcMonitor).then_some(monitor.0 as isize)
    }
}

/// Window classes of the desktop and the taskbar, which are never fullscreen apps.
fn is_shell(class: &str) -> bool {
    matches!(class, "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

fn covers(r: RECT, monitor: RECT) -> bool {
    r.left <= monitor.left && r.top <= monitor.top && r.right >= monitor.right && r.bottom >= monitor.bottom
}

fn place_of(hwnd: HWND) -> Option<Place> {
    with_state(|s| s.slot_of(hwnd).map(|i| s.slots[i].place)).flatten()
}

/// Signed x, y packed in a mouse message's lparam.
fn point_from(lparam: LPARAM) -> POINT {
    POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 }
}

fn lparam_is(lparam: LPARAM, s: &str) -> bool {
    lparam.0 != 0 && unsafe { PCWSTR(lparam.0 as *const u16).to_string() }.is_ok_and(|v| v == s)
}

fn px(dip_times_scale: f32) -> i32 {
    dip_times_scale.ceil() as i32
}

#[cfg(test)]
mod tests {
    use super::snap_axis;

    #[test]
    fn fullscreen_means_the_whole_monitor() {
        use windows::Win32::Foundation::RECT;
        let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        assert!(super::covers(monitor, monitor));
        assert!(super::covers(RECT { left: -8, top: -8, right: 1928, bottom: 1088 }, monitor), "borderless with a frame outside");
        assert!(!super::covers(RECT { left: 0, top: 0, right: 1920, bottom: 1032 }, monitor), "maximized above the taskbar");
        assert!(super::is_shell("WorkerW") && super::is_shell("Progman") && !super::is_shell("UnityWndClass"));
    }

    #[test]
    fn snapping() {
        // Left edge 5 px from the work area edge at 0: snaps.
        assert_eq!(snap_axis(5, 205, &[(0, false), (1920, true)], 12), -5);
        // Right edge 8 px short of 1920: snaps.
        assert_eq!(snap_axis(1712, 1912, &[(0, false), (1920, true)], 12), 8);
        // Nothing in reach.
        assert_eq!(snap_axis(500, 700, &[(0, false), (1920, true)], 12), 0);
        // The nearest target wins.
        assert_eq!(snap_axis(100, 300, &[(95, false), (302, true)], 12), 2);
    }
}
