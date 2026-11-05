use std::f32::consts::{FRAC_PI_2, PI};

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::core::Result;
use windows_numerics::Vector2;

/// Rounded rectangle with continuous (superellipse, n = 4) corners, the "squircle" look.
// ponytail: polyline with 24 points per corner; switch to cubic segments if large sizes show facets.
pub fn squircle(f: &ID2D1Factory1, x: f32, y: f32, w: f32, h: f32, r: f32) -> Result<ID2D1PathGeometry1> {
    const STEPS: usize = 24;
    let r = r.min(w / 2.0).min(h / 2.0);
    unsafe {
        let geo = f.CreatePathGeometry()?;
        let sink = geo.Open()?;
        // Corner centers with start angle, clockwise from top-left.
        let corners = [
            (x + r, y + r, PI),
            (x + w - r, y + r, 1.5 * PI),
            (x + w - r, y + h - r, 0.0),
            (x + r, y + h - r, 0.5 * PI),
        ];
        let mut first = true;
        for (cx, cy, a0) in corners {
            for i in 0..=STEPS {
                let (s, c) = (a0 + i as f32 / STEPS as f32 * FRAC_PI_2).sin_cos();
                let p = Vector2 { X: cx + r * c.signum() * c.abs().sqrt(), Y: cy + r * s.signum() * s.abs().sqrt() };
                if first {
                    sink.BeginFigure(p, D2D1_FIGURE_BEGIN_FILLED);
                    first = false;
                } else {
                    sink.AddLine(p);
                }
            }
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        Ok(geo)
    }
}
