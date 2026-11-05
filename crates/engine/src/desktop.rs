//! Placement in the desktop layer and per-monitor DPI.
//!
//! Widgets are child windows of the window that hosts the desktop icons (`SHELLDLL_DefView`),
//! placed above the icons. That keeps them below every app window, visible after Win+D,
//! and able to receive mouse input. Findings from the M0 spike on Windows 11 build 26300:
//! - Under the wallpaper `WorkerW`, the widget is invisible: WorkerW paints over its children.
//! - Below `SHELLDLL_DefView`, the widget is visible but gets no input: the icon layer is full-screen.
//! - Child windows of Progman need `WS_EX_LAYERED` to be composited.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

/// The window that hosts the desktop icons, or None while Explorer is not running.
pub(crate) fn icon_host() -> Option<HWND> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), None).ok()?;
        if FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), None).is_ok() {
            return Some(progman);
        }
        // Older layout after a wallpaper app ran: the icons live in a top-level WorkerW.
        let mut found = HWND::default();
        let _ = EnumWindows(Some(find_defview_host), LPARAM(&mut found as *mut HWND as isize));
        (!found.is_invalid()).then_some(found)
    }
}

/// The list view that draws the desktop icons.
pub(crate) fn icon_list() -> Option<HWND> {
    unsafe {
        let defview = FindWindowExW(icon_host(), None, w!("SHELLDLL_DefView"), None).ok()?;
        FindWindowExW(Some(defview), None, w!("SysListView32"), None).ok()
    }
}

unsafe extern "system" fn find_defview_host(top: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        if FindWindowExW(Some(top), None, w!("SHELLDLL_DefView"), None).is_ok() {
            *(lparam.0 as *mut HWND) = top;
            return false.into();
        }
    }
    true.into()
}

/// Creates a widget window at `pos` (screen pixels), above the desktop icons.
pub(crate) fn create_window(class: PCWSTR, host: HWND, pos: POINT, w: i32, h: i32) -> Result<HWND> {
    // Creating it directly as a layered child of Progman fails (CreateWindowExW returns NULL).
    // Create a popup, then turn it into a layered child. This order is the one proven in the spike.
    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            class,
            w!("Widget"),
            WS_POPUP,
            pos.x,
            pos.y,
            w,
            h,
            None,
            None,
            None,
            None,
        )?;
        SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS).0 as isize);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
        SetParent(hwnd, Some(host))?;
        let mut p = pos;
        let _ = ScreenToClient(host, &mut p);
        SetWindowPos(hwnd, Some(HWND_TOP), p.x, p.y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE)?;
        Ok(hwnd)
    }
}

/// Creates a hidden top-level window at `pos` (screen pixels) for a panel, or a topmost one for
/// a widget being dragged out. No taskbar button.
pub(crate) fn create_popup(class: PCWSTR, pos: POINT, w: i32, h: i32, topmost: bool) -> Result<HWND> {
    let ex = WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP | if topmost { WS_EX_TOPMOST } else { Default::default() };
    unsafe { CreateWindowExW(ex, class, w!("Widget"), WS_POPUP, pos.x, pos.y, w, h, None, None, None, None) }
}

/// Pixels per DIP on the monitor nearest to `pos` (screen pixels).
pub(crate) fn scale_at(pos: POINT) -> f32 {
    monitor_scale(unsafe { MonitorFromPoint(pos, MONITOR_DEFAULTTONEAREST) })
}

/// Pixels per DIP on the monitor that holds most of the window.
pub(crate) fn scale_of(hwnd: HWND) -> f32 {
    monitor_scale(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) })
}

fn monitor_scale(monitor: HMONITOR) -> f32 {
    let (mut x, mut y) = (96u32, 96u32);
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) };
    x as f32 / 96.0
}

/// Moves `r` (screen pixels) so it lies inside the work area of the monitor it mostly covers.
pub(crate) fn keep_on_screen(r: RECT) -> RECT {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let monitor = MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST);
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return r;
        }
    }
    clamp_rect(r, info.rcWork)
}

/// Work area (screen minus taskbar) of the monitor `r` mostly covers.
pub(crate) fn work_area(r: RECT) -> RECT {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = unsafe { GetMonitorInfoW(MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST), &mut info) };
    info.rcWork
}

/// Shifts `r` into `area` without resizing. Aligns to the top-left if `r` is larger.
pub(crate) fn clamp_rect(r: RECT, area: RECT) -> RECT {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let x = r.left.min(area.right - w).max(area.left);
    let y = r.top.min(area.bottom - h).max(area.top);
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}

/// Puts the card (w x h at `card`, screen pixels) inside its monitor's work area and sizes the
/// window around it with `margin` pixels on each side for the card shadow.
pub(crate) fn place(hwnd: HWND, card: POINT, w: i32, h: i32, margin: i32) -> Result<()> {
    let r = keep_on_screen(RECT { left: card.x, top: card.y, right: card.x + w, bottom: card.y + h });
    let (w, h) = (w + 2 * margin, h + 2 * margin);
    let mut p = POINT { x: r.left - margin, y: r.top - margin };
    unsafe {
        if let Ok(parent) = GetParent(hwnd) {
            let _ = ScreenToClient(parent, &mut p);
        }        SetWindowPos(hwnd, None, p.x, p.y, w, h, SWP_NOZORDER | SWP_NOACTIVATE)
    }
}

pub(crate) fn window_rect(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut r) };
    r
}

/// Top-left corner of a window in screen pixels.
pub(crate) fn screen_pos(hwnd: HWND) -> POINT {
    let mut r = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut r) };
    POINT { x: r.left, y: r.top }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    #[test]
    fn clamp() {
        let area = r(0, 0, 1920, 1040);
        assert_eq!(clamp_rect(r(100, 100, 500, 300), area), r(100, 100, 500, 300));
        assert_eq!(clamp_rect(r(1800, -50, 2200, 150), area), r(1520, 0, 1920, 200));
        assert_eq!(clamp_rect(r(-300, 1000, 100, 1200), area), r(0, 840, 400, 1040));
        // Larger than the area: pinned to the top-left.
        assert_eq!(clamp_rect(r(50, 50, 2050, 1150), area), r(0, 0, 2000, 1100));
    }
}
