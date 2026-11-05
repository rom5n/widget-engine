//! Runs a widget package with hot reload: save a file in the package and the widget updates.
//! Errors (with file and line) print here, and the last good version stays on screen.
//!
//! Usage: widget-dev <package-dir>
//!        widget-dev --schema > schemas/layout.xsd
//! Example: cargo run -p widget-engine --bin widget-dev --release -- widgets/clock

use widget_engine::Engine;
use widget_engine::windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};

fn main() {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: widget-dev <package-dir>");
        eprintln!("       widget-dev --schema   (prints the layout.xml schema)");
        std::process::exit(2);
    };
    if dir == "--schema" {
        print!("{}", widget_engine::layout_schema());
        return;
    }
    let name = std::path::Path::new(&dir).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Its own id, so position and settings do not mix with the app that ships the widget.
    let id = format!("dev-{}", name.replace(['.', '=', '\n'], "-"));
    let mut engine = Engine::new();
    let x = unsafe { GetSystemMetrics(SM_CXSCREEN) } - 520;
    if let Err(e) = engine.load(&id, &dir, x, 60) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    engine.hot_reload();
    eprintln!("watching {dir}. Click the widget and press Esc to quit.");
    if let Err(e) = engine.run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
