//! Paints (solid colors and gradients) and drop shadows for markup elements.
//!
//! - `#RRGGBB`, `accent`, ...: a solid color, see [`crate::markup::parse_color`].
//! - `linear(135deg, accent, #000 80%)`: CSS-style angle (0deg = up, 90deg = right).
//! - `radial(#fff, transparent)`: from the center to the edge of the box.
//! - `conic(0deg, red, blue, red)`: around the center, clockwise from the angle.
//!
//! Stops without a position are spread evenly. Shadows: `shadow="dx dy blur color, ..."`.

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::*;
use windows_numerics::{Matrix3x2, Vector2};

use crate::Theme;
use crate::markup::parse_color;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Paint {
    Solid(D2D1_COLOR_F),
    Linear(f32, Vec<Stop>),
    Radial(Vec<Stop>),
    Conic(f32, Vec<Stop>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stop {
    pub pos: f32,
    pub color: D2D1_COLOR_F,
}

pub(crate) fn parse_paint(s: &str, theme: &Theme) -> Option<Paint> {
    let s = s.trim();
    let Some((kind, rest)) = s.split_once('(') else { return parse_color(s, theme).map(Paint::Solid) };
    let args: Vec<&str> = rest.strip_suffix(')')?.split(',').map(str::trim).collect();
    let (angle, stops) = match args.first().and_then(|a| angle(a)) {
        Some(a) => (a, &args[1..]),
        None => (0.0, &args[..]),
    };
    let stops = parse_stops(stops, theme)?;
    Some(match kind.trim() {
        "linear" => Paint::Linear(angle, stops),
        "radial" => Paint::Radial(stops),
        "conic" => Paint::Conic(angle, stops),
        _ => return None,
    })
}

fn angle(s: &str) -> Option<f32> {
    s.strip_suffix("deg")?.trim().parse().ok()
}

fn parse_stops(args: &[&str], theme: &Theme) -> Option<Vec<Stop>> {
    if args.len() < 2 {
        return None;
    }
    let last = (args.len() - 1) as f32;
    args.iter()
        .enumerate()
        .map(|(i, a)| {
            let (color, pos) = match a.rsplit_once(' ') {
                Some((c, p)) if p.ends_with('%') => (c, p.trim_end_matches('%').parse::<f32>().ok()? / 100.0),
                _ => (*a, i as f32 / last),
            };
            Some(Stop { pos: pos.clamp(0.0, 1.0), color: parse_color(color, theme)? })
        })
        .collect()
}

/// A brush that paints `p` over a w x h box at the origin.
pub(crate) unsafe fn brush(ctx: &ID2D1DeviceContext, p: &Paint, w: f32, h: f32) -> Result<ID2D1Brush> {
    unsafe {
        let stops = |stops: &[Stop]| {
            let d2d: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|s| D2D1_GRADIENT_STOP { position: s.pos, color: s.color }).collect();
            // Interpolate in sRGB like CSS, but premultiplied: fading to transparent stays clean.
            ctx.CreateGradientStopCollection(
                &d2d,
                D2D1_COLOR_SPACE_SRGB,
                D2D1_COLOR_SPACE_SRGB,
                D2D1_BUFFER_PRECISION_8BPC_UNORM,
                D2D1_EXTEND_MODE_CLAMP,
                D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED,
            )
        };
        let center = Vector2 { X: w / 2.0, Y: h / 2.0 };
        Ok(match p {
            Paint::Solid(c) => ctx.CreateSolidColorBrush(c, None)?.cast()?,
            Paint::Linear(deg, s) => {
                let (start, end) = linear_ends(*deg, w, h);
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: start, endPoint: end };
                ctx.CreateLinearGradientBrush(&props, None, &stops(s)?)?.cast()?
            }
            Paint::Radial(s) => {
                let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES {
                    center,
                    gradientOriginOffset: Vector2 { X: 0.0, Y: 0.0 },
                    radiusX: w / 2.0,
                    radiusY: h / 2.0,
                };
                ctx.CreateRadialGradientBrush(&props, None, &stops(s)?)?.cast()?
            }
            Paint::Conic(deg, s) => conic_brush(ctx, *deg, s, w, h)?,
        })
    }
}

/// Gradient line through the center, long enough that the corners get the end colors (CSS rule).
fn linear_ends(deg: f32, w: f32, h: f32) -> (Vector2, Vector2) {
    let (sin, cos) = deg.to_radians().sin_cos();
    let half = (w * sin.abs() + h * cos.abs()) / 2.0;
    let (dx, dy) = (sin * half, -cos * half);
    (Vector2 { X: w / 2.0 - dx, Y: h / 2.0 - dy }, Vector2 { X: w / 2.0 + dx, Y: h / 2.0 + dy })
}

/// D2D has no conic gradient brush: render one on the CPU into a bitmap and stretch it over the box.
// ponytail: fixed 256x256 bitmap, angles stretch on non-square boxes. Use a custom pixel shader
// effect if large conic fills look soft.
unsafe fn conic_brush(ctx: &ID2D1DeviceContext, deg: f32, stops: &[Stop], w: f32, h: f32) -> Result<ID2D1Brush> {
    const N: u32 = 256;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 + 0.5 - N as f32 / 2.0, y as f32 + 0.5 - N as f32 / 2.0);
            // 0 = up, clockwise.
            let a = dx.atan2(-dy).to_degrees();
            let c = sample(stops, ((a - deg).rem_euclid(360.0)) / 360.0);
            let b = |v: f32| (v * c.a * 255.0).round() as u8;
            px.extend_from_slice(&[b(c.b), b(c.g), b(c.r), (c.a * 255.0).round() as u8]);
        }
    }
    unsafe {
        let bitmap = ctx.CreateBitmap(
            D2D_SIZE_U { width: N, height: N },
            Some(px.as_ptr().cast()),
            N * 4,
            &D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            },
        )?;
        let brush = ctx.CreateBitmapBrush(
            &bitmap,
            Some(&D2D1_BITMAP_BRUSH_PROPERTIES1 {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR,
            }),
            None,
        )?;
        brush.SetTransform(&Matrix3x2 { M11: w / N as f32, M12: 0.0, M21: 0.0, M22: h / N as f32, M31: 0.0, M32: 0.0 });
        brush.cast()
    }
}

/// Color at `t` in 0..=1, straight alpha.
fn sample(stops: &[Stop], t: f32) -> D2D1_COLOR_F {
    let first = stops[0];
    let last = stops[stops.len() - 1];
    if t <= first.pos {
        return first.color;
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t <= b.pos {
            let k = if b.pos > a.pos { (t - a.pos) / (b.pos - a.pos) } else { 1.0 };
            let mix = |x: f32, y: f32| x + (y - x) * k;
            let (ca, cb) = (a.color, b.color);
            return D2D1_COLOR_F { r: mix(ca.r, cb.r), g: mix(ca.g, cb.g), b: mix(ca.b, cb.b), a: mix(ca.a, cb.a) };
        }
    }
    last.color
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shadow {
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    pub color: D2D1_COLOR_F,
}

/// "dx dy blur [color], ...". The default color is black at 25%.
pub(crate) fn parse_shadows(s: &str, theme: &Theme) -> Vec<Shadow> {
    s.split(',')
        .filter_map(|one| {
            let parts: Vec<&str> = one.split_whitespace().collect();
            let n = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok());
            let color = match parts.get(3) {
                Some(c) => parse_color(c, theme)?,
                None => D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.25 },
            };
            Some(Shadow { dx: n(0)?, dy: n(1)?, blur: n(2)?.max(0.0), color })
        })
        .collect()
}

/// Room a layer needs around its element so the shadows are not clipped.
pub(crate) fn shadow_margin(shadows: &[Shadow]) -> f32 {
    // A blur of B is a Gaussian with sigma B/2. It fades out by 3 sigma.
    shadows.iter().map(|s| s.dx.abs().max(s.dy.abs()) + 1.5 * s.blur).fold(0.0f32, f32::max).ceil()
}

/// Records `content` into a command list, then draws its shadows and the content itself.
pub(crate) unsafe fn with_shadows(
    ctx: &ID2D1DeviceContext,
    shadows: &[Shadow],
    content: impl FnOnce() -> Result<()>,
) -> Result<()> {
    unsafe {
        if shadows.is_empty() {
            return content();
        }
        let transform = {
            let mut m = Matrix3x2::default();
            ctx.GetTransform(&mut m);
            m
        };
        let target = ctx.GetTarget()?;
        let list = ctx.CreateCommandList()?;
        ctx.SetTarget(&list);
        // Record in element space. Drawing the list applies the transform once.
        ctx.SetTransform(&Matrix3x2::identity());
        let drawn = content();
        ctx.SetTarget(&target);
        list.Close()?;
        drawn?;
        ctx.SetTransform(&transform);
        for s in shadows {
            let effect = ctx.CreateEffect(&CLSID_D2D1Shadow)?;
            effect.SetInput(0, &list, true);
            let sigma = s.blur / 2.0;
            effect.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &sigma.to_le_bytes())?;
            let c = s.color;
            let rgba: Vec<u8> = [c.r, c.g, c.b, c.a].iter().flat_map(|v| v.to_le_bytes()).collect();
            effect.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, &rgba)?;
            ctx.DrawImage(
                &effect.GetOutput()?,
                Some(&Vector2 { X: s.dx, Y: s.dy }),
                None,
                D2D1_INTERPOLATION_MODE_LINEAR,
                D2D1_COMPOSITE_MODE_SOURCE_OVER,
            );
        }
        ctx.DrawImage(&list, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rgba;

    fn theme() -> Theme {
        Theme { dark: true, accent: rgba(0.0, 0.5, 1.0, 1.0), text: None }
    }

    #[test]
    fn paints() {
        let t = theme();
        assert_eq!(parse_paint("#fff", &t), Some(Paint::Solid(rgba(1.0, 1.0, 1.0, 1.0))));
        let Some(Paint::Linear(a, s)) = parse_paint("linear(90deg, #000, accent 25%, #fff)", &t) else { panic!() };
        assert_eq!(a, 90.0);
        assert_eq!(s.iter().map(|s| s.pos).collect::<Vec<_>>(), [0.0, 0.25, 1.0]);
        let Some(Paint::Radial(s)) = parse_paint("radial(#fff, transparent)", &t) else { panic!() };
        assert_eq!(s.len(), 2);
        assert!(matches!(parse_paint("conic(45deg, #f00, #00f)", &t), Some(Paint::Conic(45.0, _))));
        assert_eq!(parse_paint("linear(90deg, #000)", &t), None, "one stop is not a gradient");
        assert_eq!(parse_paint("wavy(#000, #fff)", &t), None);
    }

    #[test]
    fn linear_line_reaches_corners() {
        let (s, e) = linear_ends(90.0, 200.0, 100.0);
        assert!((s.X - 0.0).abs() < 1e-3 && (e.X - 200.0).abs() < 1e-3 && (s.Y - 50.0).abs() < 1e-3);
        let (s, e) = linear_ends(180.0, 200.0, 100.0); // top to bottom
        assert!((s.Y - 0.0).abs() < 1e-3 && (e.Y - 100.0).abs() < 1e-3);
    }

    #[test]
    fn sampling() {
        let s = [Stop { pos: 0.0, color: rgba(0.0, 0.0, 0.0, 1.0) }, Stop { pos: 1.0, color: rgba(1.0, 1.0, 1.0, 1.0) }];
        assert!((sample(&s, 0.5).r - 0.5).abs() < 1e-6);
        assert_eq!(sample(&s, 2.0).r, 1.0);
    }

    #[test]
    fn shadows() {
        let s = parse_shadows("0 4 12 #00000080, 0 1 2", &theme());
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].dy, s[0].blur), (4.0, 12.0));
        assert_eq!(s[1].color.a, 0.25);
        assert_eq!(shadow_margin(&s), 22.0); // 4 + 1.5 * 12
    }
}
