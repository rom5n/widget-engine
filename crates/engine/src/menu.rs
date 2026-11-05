//! Engine-drawn context menu: rounded panels, cascading submenus, hairlines between items,
//! mouse and keyboard. Replaces the Win32 popup menu.
//!
//! One transparent topmost window covers the work area while the menu is open. Each panel is a
//! DComp visual in it. A click outside the panels, Esc, or switching apps closes the menu.

use std::cell::RefCell;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;
use windows_numerics::{Matrix3x2, Vector2};

use crate::paint::{Shadow, shadow_margin, with_shadows};
use crate::settings::Action;
use crate::theme::{Theme, rgb, rgba};

/// Layout in DIPs.
const RADIUS: f32 = 20.0;
/// Room between the panel edge and the highlight.
const PAD: f32 = 7.0;
const ROW: f32 = 32.0;
/// Panel edge to the text. Hairlines start and end this far in.
const TEXT_X: f32 = 18.0;
/// Center of the checkmark or chevron, from the right edge.
const MARK_X: f32 = 20.0;
const MIN_W: f32 = 180.0;
const FONT: &str = "Segoe UI Variable Text";
const FONT_SIZE: f32 = 14.0;
const CLASS: PCWSTR = w!("WidgetEngineMenu");

#[derive(Clone)]
pub(crate) struct Item {
    pub label: String,
    pub kind: Kind,
    pub checked: bool,
    /// Drawn in red, e.g. "Delete widget".
    pub danger: bool,
}

#[derive(Clone)]
pub(crate) enum Kind {
    Pick(Action),
    Sub(Vec<Item>),
}

impl Item {
    pub fn pick(label: &str, checked: bool, action: Action) -> Self {
        Self { label: label.into(), kind: Kind::Pick(action), checked, danger: false }
    }

    pub fn sub(label: &str, items: Vec<Item>) -> Self {
        Self { label: label.into(), kind: Kind::Sub(items), checked: false, danger: false }
    }
}

/// Device objects the menu draws with, cloned from the engine's.
pub(crate) struct Gfx {
    pub d2d: ID2D1Factory1,
    pub dwrite: IDWriteFactory,
    pub dcomp: IDCompositionDesktopDevice,
}

struct Panel {
    items: Vec<Item>,
    /// DIPs relative to the overlay window.
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    hover: Option<usize>,
    visual: IDCompositionVisual2,
    surface: IDCompositionSurface,
}

struct Menu {
    gfx: Gfx,
    _target: IDCompositionTarget,
    root: IDCompositionVisual2,
    panels: Vec<Panel>,
    scale: f32,
    /// Work area in DIPs relative to the overlay.
    area: (f32, f32, f32, f32),
    theme: Theme,
    format: IDWriteTextFormat,
    stroke: ID2D1StrokeStyle1,
    shadows: [Shadow; 2],
    /// Set when the menu closes: the pick, or None.
    done: Option<Option<Action>>,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

/// Shows the menu with its top-left corner at `at` (screen pixels). Blocks until the user
/// picks an item or dismisses the menu.
pub(crate) fn run(gfx: Gfx, at: POINT, items: Vec<Item>) -> Option<Action> {
    let hwnd = match open(gfx, at, items) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("widget engine: menu: {e}");
            return None;
        }
    };
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let mut msg = MSG::default();
        while MENU.with_borrow(|m| m.as_ref().is_some_and(|m| m.done.is_none())) {
            if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                // WM_QUIT: close the menu and let the app's loop see it.
                PostQuitMessage(msg.wParam.0 as i32);
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // Take the state out first: destroying the window sends messages to `proc`.
        let menu = MENU.take();
        let _ = DestroyWindow(hwnd);
        menu.and_then(|m| m.done.flatten())
    }
}

fn open(gfx: Gfx, at: POINT, items: Vec<Item>) -> Result<HWND> {
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
        // Fails harmlessly after the first menu.
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: instance,
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        });
        let monitor = MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        // The work area, not the whole monitor: a topmost window that covers the monitor counts
        // as a fullscreen app, and the engine hides the widgets for those.
        let m = info.rcWork;
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP,
            CLASS,
            w!(""),
            WS_POPUP,
            m.left,
            m.top,
            m.right - m.left,
            m.bottom - m.top,
            None,
            None,
            Some(instance),
            None,
        )?;
        let built = (|| {
            let scale = crate::desktop::scale_at(at);
            let target = gfx.dcomp.CreateTargetForHwnd(hwnd, true)?;
            let root = gfx.dcomp.CreateVisual()?;
            target.SetRoot(&root)?;
            let format = gfx.dwrite.CreateTextFormat(
                &HSTRING::from(FONT),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                FONT_SIZE,
                w!(""),
            )?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let stroke = gfx.d2d.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES1 {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )?;
            let theme = Theme::current();
            let k = if theme.dark { 2.0 } else { 1.0 };
            let w = info.rcWork;
            let dip = |v: i32, o: i32| (v - o) as f32 / scale;
            let mut menu = Menu {
                gfx,
                _target: target,
                root,
                panels: Vec::new(),
                scale,
                area: (dip(w.left, m.left), dip(w.top, m.top), dip(w.right, m.left), dip(w.bottom, m.top)),
                theme,
                format,
                stroke,
                shadows: [
                    Shadow { dx: 0.0, dy: 8.0, blur: 28.0, color: rgba(0.0, 0.0, 0.0, 0.16 * k) },
                    Shadow { dx: 0.0, dy: 1.0, blur: 3.0, color: rgba(0.0, 0.0, 0.0, 0.12 * k) },
                ],
                done: None,
            };
            menu.push(items, dip(at.x, m.left), dip(at.y, m.top), None)?;
            menu.gfx.dcomp.Commit()?;
            Ok(menu)
        })();
        match built {
            Ok(menu) => {
                MENU.set(Some(menu));
                Ok(hwnd)
            }
            Err(e) => {
                let _ = DestroyWindow(hwnd);
                Err(e)
            }
        }
    }
}

impl Menu {
    /// Opens a panel. Root: top-left at (x, y). Submenu (`beside` = parent's left and right
    /// edges): its first row lines up with row y, on the right of the parent if it fits.
    fn push(&mut self, items: Vec<Item>, x: f32, y: f32, beside: Option<(f32, f32)>) -> Result<()> {
        let mut text_w = 0.0f32;
        for item in &items {
            let label: Vec<u16> = item.label.encode_utf16().collect();
            let layout = unsafe { self.gfx.dwrite.CreateTextLayout(&label, &self.format, 10_000.0, ROW)? };
            let mut m = DWRITE_TEXT_METRICS::default();
            unsafe { layout.GetMetrics(&mut m)? };
            text_w = text_w.max(m.widthIncludingTrailingWhitespace);
        }
        let w = (TEXT_X + text_w + MARK_X + 16.0).ceil().max(MIN_W);
        let h = 2.0 * PAD + ROW * items.len() as f32;
        let (left, top, right, bottom) = self.area;
        let mut x = match beside {
            None if x + w > right => x - w,
            None => x,
            Some((pl, pr)) if pr - 4.0 + w > right => pl + 4.0 - w,
            Some((_, pr)) => pr - 4.0,
        };
        let mut y = match beside {
            None if y + h > bottom => y - h,
            None => y,
            Some(_) => y - PAD,
        };
        x = x.min(right - w).max(left);
        y = y.min(bottom - h).max(top);
        // Whole pixels, so hairlines stay sharp.
        let s = self.scale;
        let (x, y) = ((x * s).round() / s, (y * s).round() / s);
        let m = shadow_margin(&self.shadows);
        unsafe {
            let surface = self.gfx.dcomp.CreateSurface(
                ((w + 2.0 * m) * s).ceil() as u32,
                ((h + 2.0 * m) * s).ceil() as u32,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_ALPHA_MODE_PREMULTIPLIED,
            )?;
            let visual = self.gfx.dcomp.CreateVisual()?;
            visual.SetContent(&surface)?;
            visual.SetOffsetX2(((x - m) * s).round())?;
            visual.SetOffsetY2(((y - m) * s).round())?;
            self.root.AddVisual(&visual, false, None)?;
            self.panels.push(Panel { items, x, y, w, h, hover: None, visual, surface });
        }
        self.draw(self.panels.len() - 1)
    }

    /// Closes panels deeper than `keep`.
    fn truncate(&mut self, keep: usize) {
        while self.panels.len() > keep {
            let p = self.panels.pop().unwrap();
            let _ = unsafe { self.root.RemoveVisual(&p.visual) };
        }
    }

    /// Hovers row `row` of panel `k`, closing deeper panels and opening its submenu.
    fn hover(&mut self, k: usize, row: Option<usize>) -> Result<()> {
        if self.panels[k].hover == row && (row.is_none() || self.panels.len() > k + 1 || !self.has_sub(k, row)) {
            return Ok(());
        }
        self.truncate(k + 1);
        self.panels[k].hover = row;
        self.draw(k)?;
        if let Some(j) = row {
            let p = &self.panels[k];
            if let Kind::Sub(items) = &p.items[j].kind {
                let (items, y, edges) = (items.clone(), p.y + PAD + j as f32 * ROW, (p.x, p.x + p.w));
                self.push(items, 0.0, y, Some(edges))?;
            }
        }
        Ok(())
    }

    fn has_sub(&self, k: usize, row: Option<usize>) -> bool {
        row.is_some_and(|j| matches!(self.panels[k].items[j].kind, Kind::Sub(_)))
    }

    /// (panel, row) under a point in DIPs. Row None: the panel's padding.
    fn hit(&self, x: f32, y: f32) -> Option<(usize, Option<usize>)> {
        self.panels.iter().enumerate().rev().find_map(|(k, p)| {
            if x < p.x || x >= p.x + p.w || y < p.y || y >= p.y + p.h {
                return None;
            }
            let row = ((y - p.y - PAD) / ROW).floor();
            Some((k, (row >= 0.0 && (row as usize) < p.items.len()).then_some(row as usize)))
        })
    }

    fn choose(&mut self, k: usize, j: usize) -> Result<()> {
        match self.panels[k].items[j].kind.clone() {
            Kind::Pick(a) => self.done = Some(Some(a)),
            Kind::Sub(_) => {
                self.hover(k, Some(j))?;
                if let Some(sub) = self.panels.get_mut(k + 1) {
                    sub.hover = Some(0);
                    self.draw(k + 1)?;
                }
            }
        }
        Ok(())
    }

    fn key(&mut self, vk: VIRTUAL_KEY) -> Result<()> {
        let k = self.panels.len() - 1;
        let n = self.panels[k].items.len();
        let cur = self.panels[k].hover;
        match vk {
            VK_DOWN => self.hover(k, Some(cur.map_or(0, |j| (j + 1) % n)))?,
            VK_UP => self.hover(k, Some(cur.map_or(n - 1, |j| (j + n - 1) % n)))?,
            VK_RIGHT if self.has_sub(k, cur) => self.choose(k, cur.unwrap())?,
            VK_RETURN | VK_SPACE if cur.is_some() => self.choose(k, cur.unwrap())?,
            VK_LEFT | VK_ESCAPE if k > 0 => self.truncate(k),
            VK_ESCAPE => self.done = Some(None),
            _ => {}
        }
        Ok(())
    }

    /// Handles a window message. False: not handled.
    fn on(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
        let s = self.scale;
        let x = (lparam.0 & 0xFFFF) as i16 as f32 / s;
        let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32 / s;
        let r = match msg {
            WM_MOUSEMOVE => match self.hit(x, y) {
                Some((k, Some(j))) => self.hover(k, Some(j)),
                Some(_) => Ok(()),
                // Off the panels: un-hover the deepest panel unless the pointer left through its submenu.
                None => self.hover(self.panels.len() - 1, None),
            },
            WM_LBUTTONDOWN | WM_RBUTTONDOWN if self.hit(x, y).is_none() => {
                self.done = Some(None);
                Ok(())
            }
            WM_LBUTTONUP | WM_RBUTTONUP => match self.hit(x, y) {
                Some((k, Some(j))) => self.choose(k, j),
                _ => Ok(()),
            },
            WM_KEYDOWN => self.key(VIRTUAL_KEY(wparam.0 as u16)),
            WM_ACTIVATE if wparam.0 & 0xFFFF == WA_INACTIVE as usize => {
                self.done = Some(None);
                Ok(())
            }
            WM_CANCELMODE => {
                self.done = Some(None);
                Ok(())
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => Ok(()),
            _ => return false,
        };
        if let Err(e) = r.and_then(|_| unsafe { self.gfx.dcomp.Commit() }) {
            eprintln!("widget engine: menu: {e}");
            self.done = Some(None);
        }
        true
    }

    fn draw(&self, k: usize) -> Result<()> {
        let p = &self.panels[k];
        let (s, m) = (self.scale, shadow_margin(&self.shadows));
        let dark = self.theme.dark;
        let (bg, edge, fg, line, red) = if dark {
            (rgb(0x2A2A2C), rgba(1.0, 1.0, 1.0, 0.12), rgba(1.0, 1.0, 1.0, 0.92), rgba(1.0, 1.0, 1.0, 0.10), rgb(0xFF453A))
        } else {
            (rgb(0xFFFFFF), rgba(0.0, 0.0, 0.0, 0.10), rgba(0.0, 0.0, 0.0, 0.88), rgba(0.0, 0.0, 0.0, 0.09), rgb(0xFF3B30))
        };
        let d2d = &self.gfx.d2d;
        unsafe {
            let mut offset = POINT::default();
            let ctx: ID2D1DeviceContext = p.surface.BeginDraw(None, &mut offset)?;
            let painted = (|| {
                ctx.SetDpi(96.0 * s, 96.0 * s);
                ctx.SetTransform(&Matrix3x2::translation(offset.x as f32 / s + m, offset.y as f32 / s + m));
                ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                ctx.Clear(Some(&rgba(0.0, 0.0, 0.0, 0.0)));
                let shape = round_rect(d2d, 0.0, 0.0, p.w, p.h, RADIUS)?;
                with_shadows(&ctx, &self.shadows, || {
                    ctx.FillGeometry(&shape, &ctx.CreateSolidColorBrush(&bg, None)?, None);
                    Ok(())
                })?;
                let hair = 1.0 / s;
                let rim = round_rect(d2d, hair / 2.0, hair / 2.0, p.w - hair, p.h - hair, RADIUS)?;
                ctx.DrawGeometry(&rim, &ctx.CreateSolidColorBrush(&edge, None)?, hair, None);
                // Hover: a neutral gray pill, like macOS menus with a plain look.
                let hover = if self.theme.dark { rgba(1.0, 1.0, 1.0, 0.12) } else { rgba(0.0, 0.0, 0.0, 0.07) };
                let hover = ctx.CreateSolidColorBrush(&hover, None)?;
                let (fg, red, line) =
                    (ctx.CreateSolidColorBrush(&fg, None)?, ctx.CreateSolidColorBrush(&red, None)?, ctx.CreateSolidColorBrush(&line, None)?);
                for (j, item) in p.items.iter().enumerate() {
                    let top = PAD + j as f32 * ROW;
                    let hovered = p.hover == Some(j);
                    if hovered {
                        let pill = round_rect(d2d, PAD, top, p.w - 2.0 * PAD, ROW, RADIUS - PAD)?;
                        ctx.FillGeometry(&pill, &hover, None);
                    }
                    let ink = if item.danger { &red } else { &fg };
                    let label: Vec<u16> = item.label.encode_utf16().collect();
                    let rect = D2D_RECT_F { left: TEXT_X, top, right: p.w - MARK_X - 8.0, bottom: top + ROW };
                    ctx.DrawText(&label, &self.format, &rect, ink, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                    let (cx, cy) = (p.w - MARK_X, top + ROW / 2.0);
                    let stroke = |a: (f32, f32), b: (f32, f32)| {
                        ctx.DrawLine(Vector2 { X: a.0, Y: a.1 }, Vector2 { X: b.0, Y: b.1 }, ink, 1.6, &self.stroke)
                    };
                    match item.kind {
                        Kind::Sub(_) => {
                            stroke((cx - 2.0, cy - 4.5), (cx + 2.5, cy));
                            stroke((cx + 2.5, cy), (cx - 2.0, cy + 4.5));
                        }
                        Kind::Pick(_) if item.checked => {
                            stroke((cx - 5.0, cy + 0.5), (cx - 1.5, cy + 4.0));
                            stroke((cx - 1.5, cy + 4.0), (cx + 5.0, cy - 4.0));
                        }
                        _ => {}
                    }
                    // Hairline under the row, from the text's left edge to the same inset on the
                    // right. Hidden next to the highlight.
                    if j + 1 < p.items.len() && !hovered && p.hover != Some(j + 1) {
                        let ly = ((top + ROW) * s).round() / s - hair / 2.0;
                        ctx.DrawLine(Vector2 { X: TEXT_X, Y: ly }, Vector2 { X: p.w - TEXT_X, Y: ly }, &line, hair, None);
                    }
                }
                Ok(())
            })();
            p.surface.EndDraw()?;
            painted
        }
    }
}

extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = MENU.with(|m| match m.try_borrow_mut() {
        Ok(mut m) => m.as_mut().is_some_and(|m| m.on(msg, wparam, lparam)),
        Err(_) => false,
    });
    if handled { LRESULT(0) } else { unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) } }
}

/// Circular corners: the n = 4 squircle reads as barely rounded at menu sizes.
fn round_rect(d2d: &ID2D1Factory1, x: f32, y: f32, w: f32, h: f32, r: f32) -> Result<ID2D1RoundedRectangleGeometry> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let rect = D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
    unsafe { d2d.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect, radiusX: r, radiusY: r }) }
}
