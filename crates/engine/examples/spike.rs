//! M0 spike: D3D11 + D2D + DirectWrite + DirectComposition widget window.
//!
//! Tests three risks from PLAN.md:
//!   1. Render path: DComp surfaces, premultiplied alpha, DWrite text, DComp animation.
//!   2. Backdrop on inactive windows. Keys: 1 solid squircle, 2 Mica, 3 Acrylic, 4 accent blur.
//!   3. Desktop placement. Key P toggles bottom-z top-level window vs child of WorkerW.
//!
//! Drag the widget with the mouse. Esc quits.
//! To test the inactive state: press a key, then click the desktop and look at the widget.
//!
//! Run: cargo run -p widget-engine --example spike --release

#![windows_subsystem = "windows"]

use std::cell::{Cell, RefCell};
use std::f32::consts::{FRAC_PI_2, PI};
use std::ffi::c_void;

use windows::Win32::Foundation::*;
use windows::Win32::Globalization::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::SystemInformation::*;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;
use windows_numerics::{Matrix3x2, Vector2};

// Layout in DIPs.
const W: f32 = 340.0;
const H: f32 = 160.0;
const RADIUS: f32 = 28.0;
const DIAL_X: f32 = 268.0;
const DIAL_Y: f32 = 80.0;
const DIAL_R: f32 = 50.0;
const TIMER_ID: usize = 1;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    // Read by WM_WINDOWPOSCHANGING, which can fire while APP is borrowed.
    static KEEP_BOTTOM: Cell<bool> = const { Cell::new(true) };
}

struct App {
    hwnd: HWND,
    d2d: ID2D1Factory1,
    _dwrite: IDWriteFactory,
    _d3d: ID3D11Device,
    dcomp: IDCompositionDesktopDevice,
    target: Option<IDCompositionTarget>,
    root: IDCompositionVisual2,
    hand: IDCompositionVisual2,
    hand_rot: IDCompositionRotateTransform,
    surface: Option<IDCompositionSurface>,
    hand_surface: Option<IDCompositionSurface>,
    time_fmt: IDWriteTextFormat,
    date_fmt: IDWriteTextFormat,
    hint_fmt: IDWriteTextFormat,
    mode: u8,
    on_workerw: bool,
    scale: f32,
}

fn main() -> Result<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
        let class = w!("WidgetSpike");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        });

        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW,
            class,
            w!("Widget spike"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(instance),
            None,
        )?;

        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        let (wpx, hpx) = ((W * scale).round() as i32, (H * scale).round() as i32);
        let x = GetSystemMetrics(SM_CXSCREEN) - wpx - (40.0 * scale) as i32;
        let y = (40.0 * scale) as i32;
        SetWindowPos(hwnd, Some(HWND_BOTTOM), x, y, wpx, hpx, SWP_NOACTIVATE)?;

        let app = App::new(hwnd, scale)?;
        APP.with(|a| *a.borrow_mut() = Some(app));
        with_app(|a| {
            let _ = a.apply_mode(1);
            let _ = a.sync_hand();
            a.arm_timer();
        });
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        if std::env::args().any(|a| a == "--auto-p") {
            toggle_placement(hwnd);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

/// Runs `f` on the app state. Returns None during re-entrant messages or before init.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCHITTEST => return LRESULT(HTCAPTION as isize), // native drag to move
            // As a child of Progman the widget does not get focus on its own. Keys need focus.
            WM_NCLBUTTONDOWN => {
                let _ = SetFocus(Some(hwnd));
            }
            WM_WINDOWPOSCHANGING if KEEP_BOTTOM.get() => {
                let wp = &mut *(lparam.0 as *mut WINDOWPOS);
                wp.hwndInsertAfter = HWND_BOTTOM;
                wp.flags &= !SWP_NOZORDER;
            }
            WM_KEYDOWN => {
                let key = wparam.0 as u16;
                if key == VK_ESCAPE.0 {
                    let _ = DestroyWindow(hwnd);
                } else if (b'1' as u16..=b'4' as u16).contains(&key) {
                    with_app(|a| a.apply_mode((key - b'0' as u16) as u8));
                } else if key == b'P' as u16 {
                    toggle_placement(hwnd);
                }
                return LRESULT(0);
            }
            WM_TIMER if wparam.0 == TIMER_ID => {
                with_app(|a| {
                    let _ = a.draw();
                    let _ = a.sync_hand();
                    a.arm_timer();
                });
                return LRESULT(0);
            }
            // Clock correctness: system time changed, DST, resume from sleep.
            WM_TIMECHANGE => resync(),
            WM_POWERBROADCAST if wparam.0 == PBT_APMRESUMEAUTOMATIC as usize => resync(),
            // Theme or accent color changed.
            WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
                with_app(|a| a.apply_mode(a.mode));
            }
            WM_DPICHANGED => {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                with_app(|a| {
                    a.scale = (wparam.0 & 0xFFFF) as f32 / 96.0;
                    a.surface = None;
                    a.hand_surface = None;
                    let _ = a.draw();
                    let _ = a.sync_hand();
                });
                return LRESULT(0);
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

fn resync() {
    with_app(|a| {
        let _ = a.draw();
        let _ = a.sync_hand();
        a.arm_timer();
    });
}

impl App {
    unsafe fn new(hwnd: HWND, scale: f32) -> Result<Self> {
        unsafe {
            let mut d3d = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )?;
            let d3d: ID3D11Device = d3d.unwrap();
            let dxgi: IDXGIDevice = d3d.cast()?;

            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = d2d.CreateDevice(&dxgi)?;
            let dcomp: IDCompositionDesktopDevice = DCompositionCreateDevice2(&d2d_device)?;

            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let root = dcomp.CreateVisual()?;
            let hand = dcomp.CreateVisual()?;
            let hand_rot = dcomp.CreateRotateTransform()?;
            hand.SetTransform(&hand_rot)?;
            root.AddVisual(&hand, true, None)?;
            target.SetRoot(&root)?;

            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let fmt = |family: PCWSTR, weight, size| {
                dwrite.CreateTextFormat(
                    family,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!(""),
                )
            };
            let time_fmt = fmt(w!("Segoe UI Variable Display"), DWRITE_FONT_WEIGHT_SEMI_BOLD, 54.0)?;
            let date_fmt = fmt(w!("Segoe UI Variable Text"), DWRITE_FONT_WEIGHT_NORMAL, 15.0)?;
            let hint_fmt = fmt(w!("Segoe UI Variable Small"), DWRITE_FONT_WEIGHT_NORMAL, 10.0)?;

            Ok(Self {
                hwnd,
                d2d,
                _dwrite: dwrite,
                _d3d: d3d,
                dcomp,
                target: Some(target),
                root,
                hand,
                hand_rot,
                surface: None,
                hand_surface: None,
                time_fmt,
                date_fmt,
                hint_fmt,
                mode: 1,
                on_workerw: false,
                scale,
            })
        }
    }

    /// Timer fires at the next minute boundary. No polling between ticks.
    fn arm_timer(&self) {
        let st = unsafe { GetLocalTime() };
        let ms_left = 60_000 - (st.wSecond as u32 * 1000 + st.wMilliseconds as u32);
        unsafe { SetTimer(Some(self.hwnd), TIMER_ID, ms_left + 20, None) };
    }

    fn apply_mode(&mut self, mode: u8) -> Result<()> {
        self.mode = mode;
        let backdrop = match mode {
            2 => DWMSBT_MAINWINDOW,
            3 => DWMSBT_TRANSIENTWINDOW,
            _ => DWMSBT_NONE,
        };
        let corner = if mode == 1 { DWMWCP_DONOTROUND } else { DWMWCP_ROUND };
        let m = if matches!(mode, 2 | 3) { -1 } else { 0 };
        unsafe {
            DwmExtendFrameIntoClientArea(
                self.hwnd,
                &MARGINS { cxLeftWidth: m, cxRightWidth: m, cyTopHeight: m, cyBottomHeight: m },
            )?;
            set_dwm(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, is_dark() as i32);
            set_dwm(self.hwnd, DWMWA_SYSTEMBACKDROP_TYPE, backdrop.0);
            set_dwm(self.hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, corner.0);
        }
        set_accent_blur(self.hwnd, mode == 4, is_dark());
        self.draw()
    }

    fn draw(&mut self) -> Result<()> {
        let s = self.scale;
        let dark = is_dark();
        unsafe {
            if self.surface.is_none() {
                let surface = self.dcomp.CreateSurface(
                    (W * s).round() as u32,
                    (H * s).round() as u32,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_ALPHA_MODE_PREMULTIPLIED,
                )?;
                self.root.SetContent(&surface)?;
                self.surface = Some(surface);
            }
            let surface = self.surface.as_ref().unwrap();
            let ctx = begin(surface, s)?;
            ctx.Clear(Some(&rgba(0.0, 0.0, 0.0, 0.0)));

            let fg = if dark { rgba(1.0, 1.0, 1.0, 1.0) } else { rgba(0.06, 0.06, 0.06, 1.0) };
            let fg2 = if dark { rgba(1.0, 1.0, 1.0, 0.62) } else { rgba(0.0, 0.0, 0.0, 0.58) };
            let fg = ctx.CreateSolidColorBrush(&fg, None)?;
            let fg2 = ctx.CreateSolidColorBrush(&fg2, None)?;

            // Background: own squircle in solid mode, light tint over the system backdrop otherwise.
            if self.mode == 1 {
                let shape = squircle(&self.d2d, W, H, RADIUS)?;
                let bg = if dark { rgba(0.13, 0.13, 0.14, 0.94) } else { rgba(0.97, 0.97, 0.98, 0.94) };
                ctx.FillGeometry(&shape, &ctx.CreateSolidColorBrush(&bg, None)?, None);
                let edge = if dark { rgba(1.0, 1.0, 1.0, 0.09) } else { rgba(0.0, 0.0, 0.0, 0.08) };
                ctx.DrawGeometry(&shape, &ctx.CreateSolidColorBrush(&edge, None)?, 1.0, None);
            } else {
                let tint = if dark { rgba(0.0, 0.0, 0.0, 0.12) } else { rgba(1.0, 1.0, 1.0, 0.18) };
                ctx.FillRectangle(&rect(0.0, 0.0, W, H), &ctx.CreateSolidColorBrush(&tint, None)?);
            }

            let st = GetLocalTime();
            let time = fmt_time(&st);
            let date = fmt_date(&st);
            ctx.DrawText(&time, &self.time_fmt, &rect(24.0, 18.0, 220.0, 90.0), &fg, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            ctx.DrawText(&date, &self.date_fmt, &rect(26.0, 90.0, 220.0, 120.0), &fg2, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
            let hint: Vec<u16> = format!(
                "mode {}  ·  1 solid 2 mica 3 acrylic 4 accent  ·  P {}",
                self.mode,
                if self.on_workerw { "workerw" } else { "bottom-z" }
            )
            .encode_utf16()
            .collect();
            ctx.DrawText(&hint, &self.hint_fmt, &rect(26.0, 132.0, W - 10.0, H), &fg2, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);

            // Dial face: ring and 12 ticks. The hand is a separate visual animated by DComp.
            let center = Vector2 { X: DIAL_X, Y: DIAL_Y };
            let ring = ctx.CreateSolidColorBrush(&if dark { rgba(1.0, 1.0, 1.0, 0.10) } else { rgba(0.0, 0.0, 0.0, 0.08) }, None)?;
            ctx.FillEllipse(&D2D1_ELLIPSE { point: center, radiusX: DIAL_R, radiusY: DIAL_R }, &ring);
            for i in 0..12 {
                let a = i as f32 / 12.0 * 2.0 * PI;
                let (sa, ca) = a.sin_cos();
                let inner = if i % 3 == 0 { DIAL_R - 10.0 } else { DIAL_R - 6.0 };
                ctx.DrawLine(
                    Vector2 { X: DIAL_X + sa * inner, Y: DIAL_Y - ca * inner },
                    Vector2 { X: DIAL_X + sa * (DIAL_R - 3.0), Y: DIAL_Y - ca * (DIAL_R - 3.0) },
                    &fg2,
                    if i % 3 == 0 { 2.0 } else { 1.0 },
                    None,
                );
            }
            surface.EndDraw()?;
            self.draw_hand()?;
            self.dcomp.Commit()
        }
    }

    fn draw_hand(&mut self) -> Result<()> {
        let s = self.scale;
        let (hw, hl) = (8.0_f32, DIAL_R + 4.0); // DIPs: hand surface width and length
        unsafe {
            if self.hand_surface.is_none() {
                let surface = self.dcomp.CreateSurface(
                    (hw * s).round() as u32,
                    (hl * s).round() as u32,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_ALPHA_MODE_PREMULTIPLIED,
                )?;
                self.hand.SetContent(&surface)?;
                self.hand.SetOffsetX2((DIAL_X - hw / 2.0) * s)?;
                self.hand.SetOffsetY2((DIAL_Y - (DIAL_R - 4.0)) * s)?;
                self.hand_rot.SetCenterX2(hw / 2.0 * s)?;
                self.hand_rot.SetCenterY2((DIAL_R - 4.0) * s)?;
                self.hand_surface = Some(surface);
            }
            let surface = self.hand_surface.as_ref().unwrap();
            let ctx = begin(surface, s)?;
            ctx.Clear(Some(&rgba(0.0, 0.0, 0.0, 0.0)));
            let accent = ctx.CreateSolidColorBrush(&accent_color(), None)?;
            let pivot = DIAL_R - 4.0;
            ctx.DrawLine(Vector2 { X: hw / 2.0, Y: 2.0 }, Vector2 { X: hw / 2.0, Y: pivot }, &accent, 2.0, None);
            ctx.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: hw / 2.0, Y: pivot }, radiusX: 3.5, radiusY: 3.5 }, &accent);
            surface.EndDraw()
        }
    }

    /// Second hand sweeps 6°/s, run entirely by the compositor.
    /// Segment 1 goes from the current angle to 360°, segment 2 is 0..360° over 60 s, repeated forever.
    fn sync_hand(&self) -> Result<()> {
        unsafe {
            let st = GetLocalTime();
            let sec = st.wSecond as f64 + st.wMilliseconds as f64 / 1000.0;
            let rest = 60.0 - sec;
            let anim = self.dcomp.CreateAnimation()?;
            anim.AddCubic(0.0, (sec * 6.0) as f32, 6.0, 0.0, 0.0)?;
            anim.AddCubic(rest, 0.0, 6.0, 0.0, 0.0)?;
            anim.AddRepeat(rest + 60.0, 60.0)?;
            self.hand_rot.SetAngle(&anim)?;
            self.dcomp.Commit()
        }
    }

    /// DComp targets are bound to the HWND state, so recreate after reparenting.
    fn rebind_target(&mut self) -> Result<()> {
        unsafe {
            self.target = None;
            let target = self.dcomp.CreateTargetForHwnd(self.hwnd, true)?;
            target.SetRoot(&self.root)?;
            self.target = Some(target);
        }
        self.apply_mode(self.mode)
    }
}

/// Bottom-z top-level window <-> child of the desktop WorkerW (behind the icons).
fn toggle_placement(hwnd: HWND) {
    let to_workerw = !with_app(|a| a.on_workerw).unwrap_or(false);
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let mut pt = POINT { x: r.left, y: r.top };
        if to_workerw {
            let Some((parent, insert_after)) = find_desktop_layer() else { return };
            KEEP_BOTTOM.set(false);
            SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_CHILD | WS_VISIBLE).0 as isize);
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
            let _ = SetParent(hwnd, Some(parent));
            let _ = ScreenToClient(parent, &mut pt);
            let z = if insert_after.is_some() { SWP_NOSIZE } else { SWP_NOSIZE | SWP_NOZORDER };
            let _ = SetWindowPos(hwnd, insert_after, pt.x, pt.y, 0, 0, z | SWP_NOACTIVATE);
        } else {
            SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_POPUP | WS_VISIBLE).0 as isize);
            let _ = SetParent(hwnd, None);
            KEEP_BOTTOM.set(true);
            let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), pt.x, pt.y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        }
    }
    with_app(|a| {
        a.on_workerw = to_workerw;
        let _ = a.rebind_target();
    });
    dump_tree(hwnd);
}

/// Diagnostics: writes the desktop window tree to %TEMP%\widget-spike.log.
fn dump_tree(hwnd: HWND) {
    use std::fmt::Write as _;
    unsafe fn line(out: &mut String, indent: &str, h: HWND) {
        unsafe {
            let mut class = [0u16; 64];
            let n = GetClassNameW(h, &mut class);
            let mut r = RECT::default();
            let _ = GetWindowRect(h, &mut r);
            let _ = writeln!(
                out,
                "{indent}{:?} {} vis={} rect=({},{},{},{}) style={:X} ex={:X}",
                h.0,
                String::from_utf16_lossy(&class[..n as usize]),
                IsWindowVisible(h).as_bool(),
                r.left, r.top, r.right, r.bottom,
                GetWindowLongPtrW(h, GWL_STYLE),
                GetWindowLongPtrW(h, GWL_EXSTYLE),
            );
        }
    }
    let mut out = String::new();
    unsafe {
        out.push_str("widget:\n");
        line(&mut out, "  ", hwnd);
        if let Ok(parent) = GetParent(hwnd) {
            out.push_str("parent:\n");
            line(&mut out, "  ", parent);
        }
        if let Ok(progman) = FindWindowW(w!("Progman"), None) {
            out.push_str("progman + children (top of z-order first):\n");
            line(&mut out, "  ", progman);
            let mut c = GetWindow(progman, GW_CHILD).ok();
            while let Some(h) = c {
                line(&mut out, "    ", h);
                let mut g = GetWindow(h, GW_CHILD).ok();
                while let Some(gh) = g {
                    line(&mut out, "      ", gh);
                    g = GetWindow(gh, GW_HWNDNEXT).ok();
                }
                c = GetWindow(h, GW_HWNDNEXT).ok();
            }
        }
    }
    let _ = std::fs::write(std::env::temp_dir().join("widget-spike.log"), out);
}

/// Returns (parent, insert_after) for placing the widget between wallpaper and icons.
unsafe fn find_desktop_layer() -> Option<(HWND, Option<HWND>)> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), None).ok()?;
        // Asks Explorer to create the WorkerW layer behind the desktop icons.
        SendMessageTimeoutW(progman, 0x052C, WPARAM(0xD), LPARAM(1), SMTO_NORMAL, 1000, None);
        // Windows 11 24H2+: Progman children are SHELLDLL_DefView (icons) above WorkerW (wallpaper).
        // WorkerW's own content covers its children, and the full-screen icon layer eats all input
        // below it. So parent to Progman, above the icons: visible, clickable, still under app windows.
        if FindWindowExW(Some(progman), None, w!("WorkerW"), None).is_ok() {
            return Some((progman, Some(HWND_TOP)));
        }
        // Older layout: top-level WorkerW right after the window that hosts SHELLDLL_DefView.
        let mut found = HWND::default();
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut found as *mut HWND as isize));
        (!found.is_invalid()).then_some((found, None))
    }
}

unsafe extern "system" fn enum_cb(top: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        if FindWindowExW(Some(top), None, w!("SHELLDLL_DefView"), None).is_ok() {
            if let Ok(w) = FindWindowExW(None, Some(top), w!("WorkerW"), None) {
                *(lparam.0 as *mut HWND) = w;
                return false.into();
            }
        }
        true.into()
    }
}

// --- helpers ---------------------------------------------------------------

unsafe fn begin(surface: &IDCompositionSurface, scale: f32) -> Result<ID2D1DeviceContext> {
    unsafe {
        let mut off = POINT::default();
        let ctx: ID2D1DeviceContext = surface.BeginDraw(None, &mut off)?;
        // Draw in DIPs. The surface offset is in pixels.
        ctx.SetDpi(96.0 * scale, 96.0 * scale);
        ctx.SetTransform(&Matrix3x2::translation(off.x as f32 / scale, off.y as f32 / scale));
        // ClearType needs an opaque background, translucent surfaces need grayscale.
        ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        Ok(ctx)
    }
}

/// Rounded rect with continuous (superellipse, n = 4) corners.
fn squircle(f: &ID2D1Factory1, w: f32, h: f32, r: f32) -> Result<ID2D1PathGeometry1> {
    const STEPS: usize = 24;
    unsafe {
        let geo = f.CreatePathGeometry()?;
        let sink = geo.Open()?;
        // Corner centers with start angle, clockwise from top-left.
        let corners = [(r, r, PI), (w - r, r, 1.5 * PI), (w - r, h - r, 0.0), (r, h - r, 0.5 * PI)];
        let mut first = true;
        for (cx, cy, a0) in corners {
            for i in 0..=STEPS {
                let (s, c) = (a0 + i as f32 / STEPS as f32 * FRAC_PI_2).sin_cos();
                let p = Vector2 {
                    X: cx + r * c.signum() * c.abs().sqrt(),
                    Y: cy + r * s.signum() * s.abs().sqrt(),
                };
                if first {
                    sink.BeginFigure(p, D2D1_FIGURE_BEGIN_FILLED);
                    first = false;
                } else {
                    sink.AddLine(p);
                }
            }
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        Ok(geo)
    }
}

fn fmt_time(st: &SYSTEMTIME) -> Vec<u16> {
    let mut buf = [0u16; 64];
    let n = unsafe { GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(st), PCWSTR::null(), Some(&mut buf)) };
    buf[..(n.max(1) - 1) as usize].to_vec()
}

fn fmt_date(st: &SYSTEMTIME) -> Vec<u16> {
    let mut buf = [0u16; 128];
    let n = unsafe { GetDateFormatEx(PCWSTR::null(), DATE_LONGDATE, Some(st), PCWSTR::null(), Some(&mut buf), PCWSTR::null()) };
    buf[..(n.max(1) - 1) as usize].to_vec()
}

fn is_dark() -> bool {
    let mut v = 1u32;
    let mut size = 4u32;
    let _ = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    v == 0
}

fn accent_color() -> D2D1_COLOR_F {
    let (mut argb, mut opaque) = (0u32, BOOL(0));
    if unsafe { DwmGetColorizationColor(&mut argb, &mut opaque) }.is_err() {
        return rgba(0.0, 0.47, 0.83, 1.0);
    }
    let ch = |shift: u32| ((argb >> shift) & 0xFF) as f32 / 255.0;
    rgba(ch(16), ch(8), ch(0), 1.0)
}

unsafe fn set_dwm(hwnd: HWND, attr: DWMWINDOWATTRIBUTE, value: i32) {
    let _ = unsafe { DwmSetWindowAttribute(hwnd, attr, &value as *const i32 as *const c_void, 4) };
}

/// Undocumented `SetWindowCompositionAttribute` acrylic blur (mode 4).
fn set_accent_blur(hwnd: HWND, on: bool, dark: bool) {
    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        gradient_abgr: u32,
        animation_id: u32,
    }
    #[repr(C)]
    struct CompositionData {
        attribute: u32,
        data: *mut c_void,
        size: usize,
    }
    type SetWca = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;
    const WCA_ACCENT_POLICY: u32 = 19;
    const ACCENT_DISABLED: u32 = 0;
    const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return };
        let set: SetWca = std::mem::transmute(proc);
        let mut policy = AccentPolicy {
            state: if on { ACCENT_ENABLE_ACRYLICBLURBEHIND } else { ACCENT_DISABLED },
            flags: 2,
            gradient_abgr: if dark { 0x66_20_20_20 } else { 0x66_F3_F3_F3 },
            animation_id: 0,
        };
        let mut data = CompositionData {
            attribute: WCA_ACCENT_POLICY,
            data: &mut policy as *mut AccentPolicy as *mut c_void,
            size: size_of::<AccentPolicy>(),
        };
        let _ = set(hwnd, &mut data);
    }
}

fn rgba(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}
