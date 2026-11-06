//! Package widget in the style of the macOS Calendar month widget: widgets/calendar.
//! The month grid comes from the engine's built-in `cal.*` values, no app code.
//!
//! Run: cargo run -p widgine --example calendar --release

use widgine::Engine;
use widgine::windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};

fn main() -> widgine::windows::core::Result<()> {
    let mut engine = Engine::new();
    let x = unsafe { GetSystemMetrics(SM_CXSCREEN) } - 230;
    engine.load("calendar", concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/widgets/calendar"), x, 200)?;
    engine.run()
}
