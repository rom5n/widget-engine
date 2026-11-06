//! C ABI over `widgine`. Builds `widget.dll` (+ `widget.dll.lib`); the header is
//! `include/widget.h`. It mirrors the Rust API one-to-one with an opaque engine handle and
//! `int` error codes. All calls must come from the thread that created the engine.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

use widgine::Engine;
use widgine::windows::core::{HRESULT, Interface};

pub const WE_OK: c_int = 0;
/// A null pointer or a string that is not UTF-8.
pub const WE_INVALID_ARGUMENT: c_int = -1;
/// The widget package is invalid; `we_last_error` names the file and line.
pub const WE_LOAD_FAILED: c_int = -2;
/// Windows or the GPU failed; `we_last_error` has the message.
pub const WE_RUNTIME_ERROR: c_int = -3;
/// No widget with that id.
pub const WE_NO_WIDGET: c_int = -4;

/// Opaque engine handle.
pub struct WeEngine {
    engine: Engine,
    ids: Vec<String>,
}

pub type WeActionFn = Option<unsafe extern "C" fn(user: *mut c_void, widget: *const c_char, action: *const c_char)>;
pub type WeMovedFn = Option<unsafe extern "C" fn(user: *mut c_void, widget: *const c_char, x: c_int, y: c_int)>;
/// `context` is an `ID2D1DeviceContext*`, valid only during the call. Return 0 (S_OK) or an HRESULT.
pub type WePaintFn = Option<unsafe extern "C" fn(user: *mut c_void, context: *mut c_void, width: f32, height: f32) -> c_int>;

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn fail(code: c_int, message: impl ToString) -> c_int {
    let text = CString::new(message.to_string().replace('\0', " ")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = text);
    code
}

unsafe fn text<'a>(s: *const c_char) -> Option<&'a str> {
    if s.is_null() { None } else { unsafe { CStr::from_ptr(s) }.to_str().ok() }
}

/// Message of the last failed call on this thread. Valid until the next failing call.
#[unsafe(no_mangle)]
pub extern "C" fn we_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

/// Creates an engine. Never null.
#[unsafe(no_mangle)]
pub extern "C" fn we_engine_new() -> *mut WeEngine {
    Box::into_raw(Box::new(WeEngine { engine: Engine::new(), ids: Vec::new() }))
}

/// Frees an engine that was never passed to `we_run` / `we_start`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_engine_free(engine: *mut WeEngine) {
    if !engine.is_null() {
        drop(unsafe { Box::from_raw(engine) });
    }
}

/// Loads a widget package directory. `id` is stable across runs: it keys the saved position
/// and settings. (x, y): default top-left corner in screen pixels.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_load(engine: *mut WeEngine, id: *const c_char, dir: *const c_char, x: c_int, y: c_int) -> c_int {
    let (Some(e), Some(id), Some(dir)) = (unsafe { engine.as_mut() }, unsafe { text(id) }, unsafe { text(dir) }) else {
        return fail(WE_INVALID_ARGUMENT, "we_load: null or non-UTF-8 argument");
    };
    if id.is_empty() || id.contains(['.', '=', '\n']) || e.ids.iter().any(|i| i == id) {
        return fail(WE_INVALID_ARGUMENT, format!("we_load: bad or duplicate id '{id}'"));
    }
    match e.engine.load(id, dir, x, y) {
        Ok(()) => {
            e.ids.push(id.into());
            WE_OK
        }
        Err(err) => fail(WE_LOAD_FAILED, err.message()),
    }
}

/// Sets `{app.key}` in a package widget. Works before and while the engine runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_set(widget: *const c_char, key: *const c_char, value: *const c_char) -> c_int {
    let (Some(w), Some(k), Some(v)) = (unsafe { text(widget) }, unsafe { text(key) }, unsafe { text(value) }) else {
        return fail(WE_INVALID_ARGUMENT, "we_set: null or non-UTF-8 argument");
    };
    widgine::set(w, k, v);
    WE_OK
}

/// Receives actions from `action="..."` that the engine does not handle itself.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_on_action(engine: *mut WeEngine, f: WeActionFn, user: *mut c_void) -> c_int {
    let (Some(e), Some(f)) = (unsafe { engine.as_mut() }, f) else { return fail(WE_INVALID_ARGUMENT, "we_on_action: null") };
    e.engine.on_action(move |widget, action| {
        let (w, a) = (CString::new(widget).unwrap_or_default(), CString::new(action).unwrap_or_default());
        unsafe { f(user, w.as_ptr(), a.as_ptr()) };
    });
    WE_OK
}

/// `{app.key}` = `value` in every widget of package `package` that Calmdesk shows (see
/// `widgine::host`). Any thread; no engine needed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_host_set(package: *const c_char, key: *const c_char, value: *const c_char) -> c_int {
    let (Some(p), Some(k), Some(v)) = (unsafe { text(package) }, unsafe { text(key) }, unsafe { text(value) }) else {
        return fail(WE_INVALID_ARGUMENT, "we_host_set: null or non-UTF-8 argument");
    };
    widgine::host::set(p, k, v);
    WE_OK
}

/// Clicks in widgets of package `package` that Calmdesk shows. `f` runs on a thread of the
/// engine's own.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_host_on_action(package: *const c_char, f: WeActionFn, user: *mut c_void) -> c_int {
    let (Some(p), Some(f)) = (unsafe { text(package) }, f) else { return fail(WE_INVALID_ARGUMENT, "we_host_on_action: null") };
    // The caller owns `user` and its thread safety.
    let user = user as usize;
    widgine::host::on_action(p, move |widget, action| {
        let (w, a) = (CString::new(widget).unwrap_or_default(), CString::new(action).unwrap_or_default());
        unsafe { f(user as *mut c_void, w.as_ptr(), a.as_ptr()) };
    });
    WE_OK
}

/// Protocol version of the running Calmdesk from its last answer; 0: none heard yet.
#[unsafe(no_mangle)]
pub extern "C" fn we_host_version() -> c_int {
    widgine::host::host_version() as c_int
}

/// Receives the card's new top-left corner (screen pixels) after the user moved a widget.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_on_moved(engine: *mut WeEngine, f: WeMovedFn, user: *mut c_void) -> c_int {
    let (Some(e), Some(f)) = (unsafe { engine.as_mut() }, f) else { return fail(WE_INVALID_ARGUMENT, "we_on_moved: null") };
    e.engine.on_moved(move |widget, x, y| {
        let w = CString::new(widget).unwrap_or_default();
        unsafe { f(user, w.as_ptr(), x, y) };
    });
    WE_OK
}

/// Paint callback for `<canvas id="canvas"/>` in a package widget.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_on_paint(engine: *mut WeEngine, widget: *const c_char, canvas: *const c_char, f: WePaintFn, user: *mut c_void) -> c_int {
    let (Some(e), Some(w), Some(c), Some(f)) = (unsafe { engine.as_mut() }, unsafe { text(widget) }, unsafe { text(canvas) }, f) else {
        return fail(WE_INVALID_ARGUMENT, "we_on_paint: null or non-UTF-8 argument");
    };
    if !e.ids.iter().any(|i| i == w) {
        return fail(WE_NO_WIDGET, format!("we_on_paint: no widget '{w}'"));
    }
    e.engine.on_paint(w, c, move |ctx, width, height| HRESULT(unsafe { f(user, ctx.as_raw(), width, height) }).ok());
    WE_OK
}

/// Reloads package widgets when their files change (development).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_hot_reload(engine: *mut WeEngine) -> c_int {
    let Some(e) = (unsafe { engine.as_mut() }) else { return fail(WE_INVALID_ARGUMENT, "we_hot_reload: null") };
    e.engine.hot_reload();
    WE_OK
}

/// Shows the widgets and runs the message loop until all widgets close. Takes ownership of
/// the engine: do not use or free it afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_run(engine: *mut WeEngine) -> c_int {
    if engine.is_null() {
        return fail(WE_INVALID_ARGUMENT, "we_run: null");
    }
    let e = unsafe { Box::from_raw(engine) };
    match catch_unwind(AssertUnwindSafe(|| e.engine.run())) {
        Ok(Ok(())) => WE_OK,
        Ok(Err(err)) => fail(WE_RUNTIME_ERROR, err.message()),
        Err(_) => fail(WE_RUNTIME_ERROR, "engine panicked"),
    }
}

/// Shows the widgets and returns. The app's own message loop drives them. Takes ownership of
/// the engine. Call `we_stop` before exiting.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn we_start(engine: *mut WeEngine) -> c_int {
    if engine.is_null() {
        return fail(WE_INVALID_ARGUMENT, "we_start: null");
    }
    let e = unsafe { Box::from_raw(engine) };
    match catch_unwind(AssertUnwindSafe(|| e.engine.start())) {
        Ok(Ok(())) => WE_OK,
        Ok(Err(err)) => fail(WE_RUNTIME_ERROR, err.message()),
        Err(_) => fail(WE_RUNTIME_ERROR, "engine panicked"),
    }
}

/// Closes all widgets started with `we_start`.
#[unsafe(no_mangle)]
pub extern "C" fn we_stop() {
    widgine::stop();
}
