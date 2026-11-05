//! Feeding your package's widgets when another app shows them.
//!
//! Calmdesk, the widget gallery app, shows every package it finds on the PC (see
//! docs/authoring.md) in its own process, so [`crate::set`] in your app does not reach those
//! widgets. These functions do, from any thread of your app:
//!
//! ```no_run
//! widget_engine::host::set("com.example.weather", "temp", "21°");
//! widget_engine::host::on_action("com.example.weather", |widget, action| println!("{widget}: {action}"));
//! ```
//!
//! Values are kept and sent again when Calmdesk starts after your app. Actions run on a
//! thread of the engine's own.
//!
//! The protocol, for apps in other languages: `WM_COPYDATA` to the window of class
//! [`HOST_CLASS`], `dwData` = [`MAGIC`], `lpData` UTF-8 fields separated by U+001F, `wParam`
//! the sender's window:
//! - `set␟<package id>␟<key>␟<value>`: `{app.<key>}` in every widget of the package;
//! - `hello␟<package id>`: send the clicks in the package's widgets to the sender's window.
//!
//! Both also tell Calmdesk where to send clicks. It sends `action␟<package id>␟<widget id>␟<action>`
//! the same way, and broadcasts the registered window message [`STARTED`] when it starts:
//! send everything again then. A window that should hear it must be a top-level one (not
//! message-only).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, w};

/// `dwData` of every message: "CDK1".
pub const MAGIC: usize = 0x4344_4B31;
/// Window class of Calmdesk's message window.
pub const HOST_CLASS: &str = "Calmdesk.Tray";
/// Registered window message Calmdesk broadcasts when it starts.
pub const STARTED: &str = "Calmdesk.HostStarted";
const SEP: char = '\u{1f}';
/// Longest message taken, bytes.
const MAX: usize = 64 * 1024;
/// Posted to our own window with a boxed action, so the host's send returns at once.
const WM_ACTION: u32 = WM_APP + 1;

type Handler = Arc<dyn Fn(&str, &str) + Send + Sync>;

#[derive(Default)]
struct Client {
    /// (package, key) -> value, sent again when the host starts.
    values: BTreeMap<(String, String), String>,
    handlers: BTreeMap<String, Handler>,
}

fn client() -> &'static Mutex<Client> {
    static C: OnceLock<Mutex<Client>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// The registered [`STARTED`] message.
pub fn started_message() -> u32 {
    unsafe { RegisterWindowMessageW(&HSTRING::from(STARTED)) }
}

/// The fields of a `WM_COPYDATA` message of this protocol. None: not one.
///
/// # Safety
/// `lparam` must be the `lParam` of a `WM_COPYDATA` message being handled.
pub unsafe fn decode(lparam: LPARAM) -> Option<Vec<String>> {
    let cds = unsafe { (lparam.0 as *const COPYDATASTRUCT).as_ref()? };
    if cds.dwData != MAGIC || cds.lpData.is_null() || cds.cbData as usize > MAX {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) };
    Some(std::str::from_utf8(bytes).ok()?.split(SEP).map(String::from).collect())
}

/// Sends `fields` to the window `to`, with `from` as the reply address. False: not delivered.
pub fn send(to: HWND, from: HWND, fields: &[&str]) -> bool {
    let text = fields.join("\u{1f}");
    let cds = COPYDATASTRUCT { dwData: MAGIC, cbData: text.len() as u32, lpData: text.as_ptr() as *mut _ };
    let mut result = 0usize;
    let lparam = LPARAM(&cds as *const COPYDATASTRUCT as isize);
    unsafe { SendMessageTimeoutW(to, WM_COPYDATA, WPARAM(from.0 as usize), lparam, SMTO_ABORTIFHUNG, 1000, Some(&mut result)) }.0 != 0
}

fn host() -> Option<HWND> {
    unsafe { FindWindowW(&HSTRING::from(HOST_CLASS), None) }.ok().filter(|h| !h.is_invalid())
}

/// Our window, on a thread of its own: it gets the clicks and hears the host start.
fn window() -> HWND {
    static W: OnceLock<isize> = OnceLock::new();
    let h = *W.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || unsafe {
            let instance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
            let class = w!("WidgetEngine.HostClient");
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(client_proc), hInstance: instance, lpszClassName: class, ..Default::default() });
            // Top-level, never shown: broadcasts reach it.
            let hwnd = CreateWindowExW(Default::default(), class, w!(""), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None).unwrap_or_default();
            // An app that runs as administrator still hears the host.
            let _ = ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, None);
            let _ = ChangeWindowMessageFilterEx(hwnd, started_message(), MSGFLT_ALLOW, None);
            let _ = tx.send(hwnd.0 as isize);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
        });
        rx.recv().unwrap_or(0)
    });
    HWND(h as _)
}

extern "system" fn client_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COPYDATA => {
                let Some(f) = decode(lparam).filter(|f| f.len() == 4 && f[0] == "action") else { return LRESULT(0) };
                let _ = PostMessageW(Some(hwnd), WM_ACTION, WPARAM(0), LPARAM(Box::into_raw(Box::new(f)) as isize));
                LRESULT(1)
            }
            WM_ACTION => {
                let f = Box::from_raw(lparam.0 as *mut Vec<String>);
                let handler = client().lock().unwrap().handlers.get(&f[1]).cloned();
                if let Some(h) = handler {
                    h(&f[2], &f[3]);
                }
                LRESULT(0)
            }
            m if m == started_message() => {
                resend();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Everything again, for a host that just started.
fn resend() {
    let Some(h) = host() else { return };
    let (hellos, values): (Vec<String>, Vec<(String, String, String)>) = {
        let c = client().lock().unwrap();
        (c.handlers.keys().cloned().collect(), c.values.iter().map(|((p, k), v)| (p.clone(), k.clone(), v.clone())).collect())
    };
    let me = window();
    for p in &hellos {
        send(h, me, &["hello", p]);
    }
    for (p, k, v) in &values {
        send(h, me, &["set", p, k, v]);
    }
}

/// `{app.<key>}` = `value` in every widget of package `package` (its widget.toml `id`) that
/// Calmdesk shows, now and in widgets added later.
pub fn set(package: &str, key: &str, value: &str) {
    client().lock().unwrap().values.insert((package.into(), key.into()), value.into());
    let me = window();
    if let Some(h) = host() {
        send(h, me, &["set", package, key, value]);
    }
}

/// Calls `f(widget id, action)` for each click on an `action` (and each `drag` release) in a
/// widget of package `package` that Calmdesk shows. Runs on a thread of the engine's own.
pub fn on_action(package: &str, f: impl Fn(&str, &str) + Send + Sync + 'static) {
    client().lock().unwrap().handlers.insert(package.into(), Arc::new(f));
    let me = window();
    if let Some(h) = host() {
        send(h, me, &["hello", package]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_reads_only_our_messages() {
        let text = ["set", "com.example.w", "temp", "21°\twith a tab"].join("\u{1f}");
        let mut cds = COPYDATASTRUCT { dwData: MAGIC, cbData: text.len() as u32, lpData: text.as_ptr() as *mut _ };
        let f = unsafe { decode(LPARAM(&cds as *const _ as isize)) }.unwrap();
        assert_eq!(f, ["set", "com.example.w", "temp", "21°\twith a tab"]);
        cds.dwData = 7;
        assert!(unsafe { decode(LPARAM(&cds as *const _ as isize)) }.is_none(), "other apps' WM_COPYDATA");
        let bad = [0xFFu8, 0xFE];
        let cds = COPYDATASTRUCT { dwData: MAGIC, cbData: 2, lpData: bad.as_ptr() as *mut _ };
        assert!(unsafe { decode(LPARAM(&cds as *const _ as isize)) }.is_none(), "not UTF-8");
    }
}
