//! Package widget in the style of the macOS Calendar month widget: widgets/calendar.
//! The month grid comes from the engine's built-in `cal.*` values, no app code.
//!
//! Run: cargo run -p widget-engine --example calendar --release

use widget_engine::Engine;
use widget_engine::windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};

fn main() -> widget_engine::windows::core::Result<()> {
    let mut engine = Engine::new();
    let x = unsafe { GetSystemMetrics(SM_CXSCREEN) } - 230;
    engine.load("calendar", concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/widgets/calendar"), x, 200)?;
    engine.run()
}
