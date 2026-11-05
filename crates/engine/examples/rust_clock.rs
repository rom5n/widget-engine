//! Clock written directly against the Rust API (Widget trait, layers, D2D drawing).
//! The reference clock is the package in widgets/clock, see examples/clock.rs.
//!
//! Run: cargo run -p widget-engine --example rust_clock --release
//! Drag to move. Right-click for settings. Click it and press Esc to quit.

use widget_engine::windows::Win32::Graphics::Direct2D::Common::*;
use widget_engine::windows::Win32::Graphics::Direct2D::*;
use widget_engine::windows::Win32::Graphics::DirectWrite::*;
use widget_engine::windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};
use widget_engine::windows::core::*;
use widget_engine::windows_numerics::Vector2;
use widget_engine::{Allow, Card, Customization, Engine, Event, Font, LayerId, Scene, Widget, rgba, time, toggle};

// Layout in DIPs.
const W: f32 = 340.0;
const H: f32 = 160.0;
const DIAL_X: f32 = 268.0;
const DIAL_Y: f32 = 80.0;
const DIAL_R: f32 = 50.0;
const HAND_W: f32 = 8.0;
const PIVOT: f32 = DIAL_R - 4.0;

/// Layers above the engine-drawn card: dial (static), text (once a minute), second hand (compositor).
#[derive(Default)]
struct Clock {
    text: Option<LayerId>,
    hand: Option<LayerId>,
    time_fmt: Option<IDWriteTextFormat>,
    date_fmt: Option<IDWriteTextFormat>,
}

impl Widget for Clock {
    fn size(&self) -> (f32, f32) {
        (W, H)
    }

    fn customization(&self) -> Customization {
        Customization {
            card: Some(Card { radius: 28.0, ..Default::default() }),
            allow: Allow::ALL,
            font: Font { weight: DWRITE_FONT_WEIGHT_SEMI_BOLD, ..Default::default() },
            options: vec![
                toggle("dial", "Dial", true),
                toggle("hand", "Second hand", true),
                toggle("date", "Date", true),
            ],
        }
    }

    fn build(&mut self, scene: &mut Scene) -> Result<()> {
        let theme = *scene.theme();
        let dark = theme.dark;
        let dial = scene.option("dial") == "on";

        if dial {
            let face = scene.add_layer(DIAL_X - DIAL_R, DIAL_Y - DIAL_R, DIAL_R * 2.0, DIAL_R * 2.0)?;
            scene.draw(face, |ctx| unsafe {
                let ring = if dark { rgba(1.0, 1.0, 1.0, 0.10) } else { rgba(0.0, 0.0, 0.0, 0.07) };
                let c = Vector2 { X: DIAL_R, Y: DIAL_R };
                ctx.FillEllipse(
                    &D2D1_ELLIPSE { point: c, radiusX: DIAL_R, radiusY: DIAL_R },
                    &ctx.CreateSolidColorBrush(&ring, None)?,
                );
                let tick = ctx.CreateSolidColorBrush(&secondary(dark), None)?;
                for i in 0..12 {
                    let (sin, cos) = (i as f32 / 12.0 * std::f32::consts::TAU).sin_cos();
                    let inner = if i % 3 == 0 { DIAL_R - 10.0 } else { DIAL_R - 6.0 };
                    let outer = DIAL_R - 3.0;
                    ctx.DrawLine(
                        Vector2 { X: c.X + sin * inner, Y: c.Y - cos * inner },
                        Vector2 { X: c.X + sin * outer, Y: c.Y - cos * outer },
                        &tick,
                        if i % 3 == 0 { 2.0 } else { 1.0 },
                        None,
                    );
                }
                Ok(())
            })?;
        }

        self.time_fmt = Some(scene.text_format(56.0, None)?);
        self.date_fmt = Some(scene.text_format(15.0, Some(DWRITE_FONT_WEIGHT_NORMAL))?);
        let text_w = if dial { 220.0 } else { W };
        self.text = Some(scene.add_layer(0.0, 0.0, text_w, H)?);

        // Second hand: drawn once, rotated by DComp around its pivot.
        self.hand = None;
        if dial && scene.option("hand") == "on" {
            let hand = scene.add_layer(DIAL_X - HAND_W / 2.0, DIAL_Y - PIVOT, HAND_W, PIVOT + 4.0)?;
            scene.draw(hand, |ctx| unsafe {
                let accent = ctx.CreateSolidColorBrush(&theme.accent, None)?;
                let x = HAND_W / 2.0;
                ctx.DrawLine(Vector2 { X: x, Y: 2.0 }, Vector2 { X: x, Y: PIVOT }, &accent, 2.0, None);
                ctx.FillEllipse(
                    &D2D1_ELLIPSE { point: Vector2 { X: x, Y: PIVOT }, radiusX: 3.5, radiusY: 3.5 },
                    &accent,
                );
                Ok(())
            })?;
            self.hand = Some(hand);
        }

        self.tick(scene)
    }

    fn on_event(&mut self, event: Event, scene: &mut Scene) -> Result<()> {
        match event {
            Event::Timer | Event::TimeChanged => self.tick(scene),
            _ => Ok(()),
        }
    }
}

impl Clock {
    /// Redraws the text, re-syncs the second hand, and sleeps until the next minute.
    fn tick(&mut self, scene: &mut Scene) -> Result<()> {
        let (Some(text), Some(time_fmt), Some(date_fmt)) = (self.text, &self.time_fmt, &self.date_fmt) else {
            return Ok(());
        };
        let now = time::now();
        let dark = scene.theme().dark;
        let date = scene.option("date") == "on";
        let right = if scene.option("dial") == "on" { 220.0 } else { W - 24.0 };
        scene.draw(text, |ctx| unsafe {
            let fg = if dark { rgba(1.0, 1.0, 1.0, 1.0) } else { rgba(0.06, 0.06, 0.06, 1.0) };
            let fg = ctx.CreateSolidColorBrush(&fg, None)?;
            let fg2 = ctx.CreateSolidColorBrush(&secondary(dark), None)?;
            let none = D2D1_DRAW_TEXT_OPTIONS_NONE;
            let mode = DWRITE_MEASURING_MODE_NATURAL;
            let top = if date { 26.0 } else { 44.0 };
            ctx.DrawText(&time::format_time(&now, false), time_fmt, &rect(24.0, top, right, top + 74.0), &fg, none, mode);
            if date {
                ctx.DrawText(&time::format_date(&now), date_fmt, &rect(26.0, 102.0, right, 130.0), &fg2, none, mode);
            }
            Ok(())
        })?;
        if let Some(hand) = self.hand {
            scene.spin(hand, HAND_W / 2.0, PIVOT, time::seconds_in_minute(&now) * 6.0, 6.0)?;
        }
        // +20 ms: a timer that fires a tick early would redraw the old minute.
        scene.set_timer(time::ms_until_next(&now, 60_000) + 20);
        Ok(())
    }
}

fn secondary(dark: bool) -> D2D1_COLOR_F {
    if dark { rgba(1.0, 1.0, 1.0, 0.65) } else { rgba(0.0, 0.0, 0.0, 0.6) }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}

fn main() -> Result<()> {
    let mut engine = Engine::new();
    let x = unsafe { GetSystemMetrics(SM_CXSCREEN) } - 480;
    engine.add("clock", Clock::default(), x, 50);
    engine.run()
}
