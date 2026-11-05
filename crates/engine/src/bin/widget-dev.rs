//! Runs a widget package with hot reload: save a file in the package and the widget updates.
//! Errors (with file and line) print here, and the last good version stays on screen.
//!
//! Usage: widget-dev <package-dir>
//!        widget-dev new <package-dir>   (a new package to start from)
//!        widget-dev --schema > schemas/layout.xsd
//! Example: cargo run -p widget-engine --bin widget-dev --release -- widgets/clock

use widget_engine::Engine;
use widget_engine::windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};

fn main() {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: widget-dev <package-dir>");
        eprintln!("       widget-dev new <package-dir>   (creates a package to start from)");
        eprintln!("       widget-dev --schema   (prints the layout.xml schema)");
        std::process::exit(2);
    };
    if dir == "--schema" {
        print!("{}", widget_engine::layout_schema());
        return;
    }
    if dir == "new" {
        let Some(target) = std::env::args().nth(2) else {
            eprintln!("usage: widget-dev new <package-dir>");
            std::process::exit(2);
        };
        if let Err(e) = new_package(std::path::Path::new(&target)) {
            eprintln!("{target}: {e}");
            std::process::exit(1);
        }
        eprintln!("created {target}. Preview it: widget-dev {target}");
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

/// A package folder with a clock-like widget.toml and layout.xml. Never overwrites.
fn new_package(dir: &std::path::Path) -> std::io::Result<()> {
    if dir.join("widget.toml").exists() {
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "already a package"));
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "hello".into());
    let slug: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    std::fs::create_dir_all(dir.join("assets"))?;
    std::fs::write(
        dir.join("widget.toml"),
        format!(
            r#"id = "com.example.{slug}"
name = "{name}"
category = "Other"
engine = "^0.1"
size = [170, 170]

[card]
radius = 26

[customize]
allow = ["size", "background", "theme", "font", "shape", "colors"]
"#
        ),
    )?;
    std::fs::write(
        dir.join("layout.xml"),
        r#"<widget>
  <column width="fill" height="fill" align="center" justify="center" gap="4">
    <text value="{clock.time}" size="40" weight="700" features="tnum"/>
    <text value="{clock.weekday}" size="13" color="secondary"/>
  </column>
</widget>
"#,
    )
}
