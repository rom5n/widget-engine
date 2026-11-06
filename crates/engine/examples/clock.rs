//! Reference clock: the package in widgets/clock (widget.toml, layout.xml, SVG assets). No widget code.
//!
//! Run: cargo run -p widgine --example clock --release
//! Drag to move. Right-click for settings. Click it and press Esc to quit.

use widgine::Engine;
use widgine::windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};

fn main() -> widgine::windows::core::Result<()> {
    let mut engine = Engine::new();
    let x = unsafe { GetSystemMetrics(SM_CXSCREEN) } - 480;
    engine.load("clock", concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/widgets/clock"), x, 50)?;
    engine.run()
}
