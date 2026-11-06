//! Package widget with app-drawn content: widgets/sparkline draws its chart through a
//! `<canvas id="chart"/>` paint callback, and its Refresh button calls back into the app.
//! It also shows liquid glass, a card shadow, hover/pressed button states, and an app value
//! (`{app.peak}`) set with `Engine::set` and `widgine::set`.
//!
//! Run: cargo run -p widgine --example sparkline --release

use std::cell::RefCell;
use std::rc::Rc;

use widgine::windows::Win32::Graphics::Direct2D::Common::*;
use widgine::windows::Win32::Graphics::Direct2D::*;
use widgine::windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};
use widgine::windows_numerics::Vector2;
use widgine::{Engine, rgba};

/// Some made-up activity data. A real app would fill this from its own source.
fn sample(seed: u32) -> Vec<f32> {
    let mut x = seed.wrapping_mul(2_654_435_761) | 1;
    (0..32)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let noise = (x % 1000) as f32 / 1000.0;
            0.5 + 0.3 * (i as f32 / 4.0).sin() + 0.2 * (noise - 0.5)
        })
        .collect()
}

fn main() -> widgine::windows::core::Result<()> {
    let mut engine = Engine::new();
    let x = unsafe { GetSystemMetrics(SM_CXSCREEN) } - 480;
    engine.load("sparkline", concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/widgets/sparkline"), x, 420)?;

    let data = Rc::new(RefCell::new((1u32, sample(1))));
    let peak = |points: &[f32]| format!("{:.0}%", points.iter().fold(0.0f32, |a, &b| a.max(b)) * 100.0);
    engine.set("sparkline", "peak", &peak(&data.borrow().1));
    let chart = data.clone();
    engine.on_paint("sparkline", "chart", move |ctx, w, h| unsafe {
        let d2d: ID2D1Factory = ctx.GetFactory()?;
        let points = &chart.borrow().1;
        let at = |i: usize| Vector2 { X: w * i as f32 / (points.len() - 1) as f32, Y: h * (1.0 - points[i]) };
        let path = d2d.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(at(0), D2D1_FIGURE_BEGIN_HOLLOW);
        for i in 1..points.len() {
            sink.AddLine(at(i));
        }
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()?;
        let brush = ctx.CreateSolidColorBrush(&rgba(1.0, 1.0, 1.0, 0.95), None)?;
        let style = d2d.CreateStrokeStyle(
            &D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                ..Default::default()
            },
            None,
        )?;
        ctx.DrawGeometry(&path, &brush, 2.0, &style);
        Ok(())
    });
    engine.on_action(move |widget, action| {
        if action == "refresh" {
            let mut d = data.borrow_mut();
            d.0 += 1;
            d.1 = sample(d.0);
            // App values can change at any time while the engine runs.
            widgine::set(widget, "peak", &peak(&d.1));
        }
        eprintln!("{widget}: {action}");
    });
    engine.run()
}
