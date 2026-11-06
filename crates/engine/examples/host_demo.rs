//! Feeds examples/host-demo while Calmdesk shows it: a counter, and Reset sets it back to 0.
//!
//! 1. Make Calmdesk find the package, e.g. copy examples/host-demo to %LOCALAPPDATA%\Widgets.
//! 2. Add "Host Demo" from the gallery.
//! 3. Run: cargo run -p widgine --example host_demo --release

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use widgine::host;

const PACKAGE: &str = "com.example.host-demo";
static COUNT: AtomicU32 = AtomicU32::new(0);

fn main() {
    host::on_action(PACKAGE, |widget, action| {
        println!("{widget}: {action}");
        if action == "reset" {
            COUNT.store(0, Ordering::Relaxed);
            host::set(PACKAGE, "status", "reset");
        }
    });
    host::set(PACKAGE, "status", "counting");
    loop {
        host::set(PACKAGE, "count", &COUNT.fetch_add(1, Ordering::Relaxed).to_string());
        std::thread::sleep(Duration::from_secs(1));
    }
}
