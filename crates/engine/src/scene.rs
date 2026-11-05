//! A widget's composition tree: layers, each a DComp visual with its own surface.
//! Redrawing one layer never touches the others.

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectComposition::*;
use std::collections::BTreeMap;

use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::{Interface, Result};
use windows_numerics::Matrix3x2;

use crate::settings::Font;
use crate::Theme;
use crate::gpu::Gpu;
use crate::theme::rgba;

/// Handle to a layer created with [`Scene::add_layer`]. Valid until the next rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerId(usize);

struct Layer {
    visual: IDCompositionVisual2,
    surface: IDCompositionSurface,
    /// Scale and rotation around the layer center, created on first use.
    transforms: Option<(IDCompositionScaleTransform, IDCompositionRotateTransform)>,
    opacity: Tween,
    scale: Tween,
}

/// How a compositor property moves to a new value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Transition {
    /// Jump.
    #[default]
    None,
    /// Ease-out cubic over `ms` milliseconds.
    Ease { ms: u32 },
    /// Damped spring: quick, with a small overshoot. Settles in about 0.6 s.
    Spring,
}

impl Transition {
    /// "200" (ms, ease-out), "spring", or "none".
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "spring" => Self::Spring,
            v => v.trim_end_matches("ms").parse().map(|ms| Self::Ease { ms }).unwrap_or(Self::None),
        }
    }

    fn duration(self) -> f64 {
        match self {
            Self::None => 0.0,
            Self::Ease { ms } => ms as f64 / 1000.0,
            Self::Spring => SPRING_SECS,
        }
    }

    /// Progress (0 at start, 1 at rest) after `t` seconds.
    pub(crate) fn progress(self, t: f64) -> f64 {
        let d = self.duration();
        if t >= d {
            return 1.0;
        }
        match self {
            Self::None => 1.0,
            Self::Ease { .. } => 1.0 - (1.0 - t / d).powi(3),
            Self::Spring => {
                // Underdamped spring, stiffness 300, damping 20, mass 1.
                let w0 = 300f64.sqrt();
                let zeta = 20.0 / (2.0 * w0);
                let wd = w0 * (1.0 - zeta * zeta).sqrt();
                1.0 - (-zeta * w0 * t).exp() * ((wd * t).cos() + zeta * w0 / wd * (wd * t).sin())
            }
        }
    }
}

const SPRING_SECS: f64 = 0.6;

/// A compositor value moving from `from` to `to`, so the next change starts where it is now.
#[derive(Clone, Copy, Default)]
struct Tween {
    set: bool,
    from: f32,
    to: f32,
    start: Option<std::time::Instant>,
    how: Transition,
}

impl Tween {
    fn now(&self) -> f32 {
        let Some(start) = self.start else { return self.to };
        let p = self.how.progress(start.elapsed().as_secs_f64()) as f32;
        self.from + (self.to - self.from) * p
    }
}

/// DComp animation from `from` to `to`. Springs are sampled into short linear segments.
unsafe fn animation(dcomp: &IDCompositionDesktopDevice, from: f32, to: f32, how: Transition) -> Result<IDCompositionAnimation> {
    unsafe {
        let anim = dcomp.CreateAnimation()?;
        let d = (to - from) as f64;
        let secs = how.duration();
        match how {
            Transition::Ease { .. } => {
                // from + d * (1 - (1 - u)^3) = from + d * (3u - 3u^2 + u^3), u = t / secs.
                let (a, b, c) = (3.0 * d / secs, -3.0 * d / secs.powi(2), d / secs.powi(3));
                anim.AddCubic(0.0, from, a as f32, b as f32, c as f32)?;
            }
            _ => {
                const STEP: f64 = 1.0 / 60.0;
                let mut t = 0.0;
                while t < secs {
                    let (v0, v1) = (how.progress(t), how.progress(t + STEP));
                    anim.AddCubic(t, from + (d * v0) as f32, (d * (v1 - v0) / STEP) as f32, 0.0, 0.0)?;
                    t += STEP;
                }
            }
        }
        anim.End(secs, to)?;
        Ok(anim)
    }
}

pub struct Scene {
    dcomp: IDCompositionDesktopDevice,
    d2d: ID2D1Factory1,
    dwrite: IDWriteFactory,
    pub(crate) root: IDCompositionVisual2,
    layers: Vec<Layer>,
    scale: f32,
    size: (f32, f32),
    theme: Theme,
    font: Font,
    options: BTreeMap<String, String>,
    pub(crate) timer: Option<u32>,
    /// Bottom layer with the engine-drawn card, if the widget has one.
    pub(crate) card: Option<LayerId>,
    pub(crate) rebuild_requested: bool,
    pub(crate) actions: Vec<String>,
    pub(crate) previews: Vec<Preview>,
}

/// A request from [`Scene::add_preview`], built by the engine after [`crate::Widget::build`].
pub(crate) struct Preview {
    pub dir: std::path::PathBuf,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub values: Vec<(String, String)>,
}

/// Sets `tween` to `to`. The first value and `Transition::None` jump, later ones animate
/// from wherever the previous animation is now.
fn retarget(
    tween: &mut Tween,
    to: f32,
    how: Transition,
    jump: impl FnOnce(f32) -> Result<()>,
    animate: impl FnOnce((f32, f32)) -> Result<()>,
) -> Result<()> {
    if tween.set && tween.to == to {
        return Ok(());
    }
    let from = tween.now();
    let first = !tween.set;
    *tween = Tween { set: true, from, to, start: None, how };
    if first || how == Transition::None || from == to {
        return jump(to);
    }
    tween.start = Some(std::time::Instant::now());
    animate((from, to))
}

impl Scene {
    pub(crate) fn new(
        gpu: &Gpu,
        scale: f32,
        size: (f32, f32),
        theme: Theme,
        font: Font,
        options: BTreeMap<String, String>,
    ) -> Result<Self> {
        Ok(Self {
            dcomp: gpu.dcomp.clone(),
            d2d: gpu.d2d.clone(),
            dwrite: gpu.dwrite.clone(),
            root: unsafe { gpu.dcomp.CreateVisual()? },
            layers: Vec::new(),
            scale,
            size,
            theme,
            font,
            options,
            timer: None,
            card: None,
            rebuild_requested: false,
            actions: Vec::new(),
            previews: Vec::new(),
        })
    }

    /// Draws the package widget at `dir` scaled down to fit (x, y, w, h), DIPs, centered, on top
    /// of this scene's layers. It uses default settings and `values` as its `{app.*}` values, and
    /// is static: no timers, no input. For galleries. Only from [`crate::Widget::build`].
    pub fn add_preview(&mut self, dir: &std::path::Path, x: f32, y: f32, w: f32, h: f32, values: &[(&str, &str)]) {
        let values = values.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        self.previews.push(Preview { dir: dir.to_path_buf(), x, y, w, h, values });
    }

    /// Asks the engine for a fresh scene and a new [`crate::Widget::build`] after this update,
    /// e.g. when the layout changed.
    pub fn request_rebuild(&mut self) {
        self.rebuild_requested = true;
    }

    /// Sets a fixed rotation of `deg` around (cx, cy) in layer DIPs. Stops any spin.
    pub fn rotate(&mut self, id: LayerId, cx: f32, cy: f32, deg: f32) -> Result<()> {
        let s = self.scale;
        let (_, rot) = self.transforms(id)?;
        unsafe {
            rot.SetCenterX2(cx * s)?;
            rot.SetCenterY2(cy * s)?;
            rot.SetAngle2(deg)
        }
    }

    fn transforms(&mut self, id: LayerId) -> Result<(IDCompositionScaleTransform, IDCompositionRotateTransform)> {
        let layer = &mut self.layers[id.0];
        if let Some(t) = &layer.transforms {
            return Ok(t.clone());
        }
        unsafe {
            let scale = self.dcomp.CreateScaleTransform()?;
            let rot = self.dcomp.CreateRotateTransform()?;
            let group = self.dcomp.CreateTransformGroup(&[Some(scale.cast()?), Some(rot.cast()?)])?;
            layer.visual.SetTransform(&group)?;
            layer.transforms = Some((scale.clone(), rot.clone()));
            Ok((scale, rot))
        }
    }

    /// Layer opacity, applied by the compositor (no redraw). Moves there with `how`.
    pub fn set_opacity(&mut self, id: LayerId, opacity: f32, how: Transition) -> Result<()> {
        let visual: IDCompositionVisual3 = self.layers[id.0].visual.cast()?;
        let dcomp = self.dcomp.clone();
        let tween = &mut self.layers[id.0].opacity;
        retarget(tween, opacity.clamp(0.0, 1.0), how, |v| unsafe { visual.SetOpacity2(v) }, |a| unsafe {
            visual.SetOpacity(&animation(&dcomp, a.0, a.1, how)?)
        })
    }

    /// Opening animation for the whole widget, card and shadow included: fades in from
    /// transparent and grows from `from` (e.g. 0.94) to full size around its center. `size` is
    /// the widget size in DIPs. For panels; call it from [`crate::Widget::build`].
    pub fn appear(&mut self, size: (f32, f32), from: f32, how: Transition) -> Result<()> {
        let s = self.scale;
        unsafe {
            let root: IDCompositionVisual3 = self.root.cast()?;
            root.SetOpacity(&animation(&self.dcomp, 0.0, 1.0, how)?)?;
            let scale = self.dcomp.CreateScaleTransform()?;
            scale.SetCenterX2(size.0 * s / 2.0)?;
            scale.SetCenterY2(size.1 * s / 2.0)?;
            let grow = animation(&self.dcomp, from, 1.0, how)?;
            scale.SetScaleX(&grow)?;
            scale.SetScaleY(&grow)?;
            self.root.SetTransform(&scale)
        }
    }

    /// Scales a layer by `k` around (cx, cy) in layer DIPs. Moves there with `how`.
    pub fn set_scale(&mut self, id: LayerId, cx: f32, cy: f32, k: f32, how: Transition) -> Result<()> {
        let s = self.scale;
        let (scale, _) = self.transforms(id)?;
        unsafe {
            scale.SetCenterX2(cx * s)?;
            scale.SetCenterY2(cy * s)?;
        }
        let dcomp = self.dcomp.clone();
        let tween = &mut self.layers[id.0].scale;
        retarget(
            tween,
            k,
            how,
            |v| unsafe {
                scale.SetScaleX2(v)?;
                scale.SetScaleY2(v)
            },
            |a| unsafe {
                let anim = animation(&dcomp, a.0, a.1, how)?;
                scale.SetScaleX(&anim)?;
                scale.SetScaleY(&anim)
            },
        )
    }

    /// Fires an action, e.g. from a click. Built in: `open:<url or path>`, `toggle:<option>`,
    /// `set:<option>=<value>`. Anything else goes to [`crate::Engine::on_action`].
    pub fn action(&mut self, name: &str) {
        self.actions.push(name.to_string());
    }

    /// Widget size in DIPs after the user's options (an option can resize it).
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// Font after the user's choices (or the widget's default font).
    pub fn font(&self) -> &Font {
        &self.font
    }

    /// Text format in the user's font. `weight` overrides the font weight, e.g. for secondary text.
    pub fn text_format(&self, size: f32, weight: Option<DWRITE_FONT_WEIGHT>) -> Result<IDWriteTextFormat> {
        let f = &self.font;
        crate::fonts::text_format(&self.dwrite, &f.family, weight.unwrap_or(f.weight), f.stretch, size)
    }

    /// Current value of a widget option declared in [`crate::Customization::options`].
    /// Panics if `key` was not declared.
    pub fn option(&self, key: &str) -> &str {
        self.options.get(key).unwrap_or_else(|| panic!("option '{key}' is not declared")).as_str()
    }

    /// Adds a layer on top of the existing ones. Position and size in DIPs.
    pub fn add_layer(&mut self, x: f32, y: f32, w: f32, h: f32) -> Result<LayerId> {
        let s = self.scale;
        unsafe {
            let surface = self.dcomp.CreateSurface(
                (w * s).ceil().max(1.0) as u32,
                (h * s).ceil().max(1.0) as u32,
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_ALPHA_MODE_PREMULTIPLIED,
            )?;
            let visual = self.dcomp.CreateVisual()?;
            visual.SetContent(&surface)?;
            visual.SetOffsetX2((x * s).round())?;
            visual.SetOffsetY2((y * s).round())?;
            // With no reference visual, insertAbove = false puts it on top of all siblings.
            self.root.AddVisual(&visual, false, None)?;
            self.layers.push(Layer { visual, surface, transforms: None, opacity: Tween::default(), scale: Tween::default() });
        }
        Ok(LayerId(self.layers.len() - 1))
    }

    /// Redraws one layer. Coordinates inside `paint` are DIPs relative to the layer.
    /// The context starts cleared to transparent, with a transform that maps to the surface.
    /// If `paint` sets its own transform, multiply it with the one from `GetTransform`.
    pub fn draw(&mut self, id: LayerId, paint: impl FnOnce(&ID2D1DeviceContext) -> Result<()>) -> Result<()> {
        let s = self.scale;
        let surface = &self.layers[id.0].surface;
        unsafe {
            let mut offset = POINT::default();
            let ctx: ID2D1DeviceContext = surface.BeginDraw(None, &mut offset)?;
            ctx.SetDpi(96.0 * s, 96.0 * s);
            ctx.SetTransform(&Matrix3x2::translation(offset.x as f32 / s, offset.y as f32 / s));
            // ClearType needs an opaque background. Widget surfaces are translucent.
            ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            ctx.Clear(Some(&rgba(0.0, 0.0, 0.0, 0.0)));
            let painted = paint(&ctx);
            surface.EndDraw()?;
            painted
        }
    }

    /// Rotates a layer forever at `deg_per_sec`, starting at `start_deg`, around (cx, cy) in layer DIPs.
    /// The compositor runs it: no CPU work per frame. Call again to re-sync.
    pub fn spin(&mut self, id: LayerId, cx: f32, cy: f32, start_deg: f32, deg_per_sec: f32) -> Result<()> {
        assert!(deg_per_sec > 0.0, "deg_per_sec must be positive");
        let s = self.scale;
        let (_, rot) = self.transforms(id)?;
        let dcomp = &self.dcomp;
        unsafe {
            rot.SetCenterX2(cx * s)?;
            rot.SetCenterY2(cy * s)?;
            // Segment 1: start_deg -> 360. Segment 2: 0 -> 360, repeated forever.
            let start = start_deg.rem_euclid(360.0);
            let period = 360.0 / deg_per_sec as f64;
            let first = (360.0 - start as f64) / deg_per_sec as f64;
            let anim = dcomp.CreateAnimation()?;
            anim.AddCubic(0.0, start, deg_per_sec, 0.0, 0.0)?;
            anim.AddCubic(first, 0.0, deg_per_sec, 0.0, 0.0)?;
            anim.AddRepeat(first + period, period)?;
            rot.SetAngle(&anim)?;
        }
        Ok(())
    }

    /// Requests one [`crate::Event::Timer`] after `ms`. Replaces any pending timer of this widget.
    pub fn set_timer(&mut self, ms: u32) {
        self.timer = Some(ms);
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Pixels per DIP on the widget's monitor.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// For geometries, effects, and other D2D resources.
    pub fn d2d(&self) -> &ID2D1Factory1 {
        &self.d2d
    }

    /// For text formats and text layouts.
    pub fn dwrite(&self) -> &IDWriteFactory {
        &self.dwrite
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions() {
        assert_eq!(Transition::parse("200"), Transition::Ease { ms: 200 });
        assert_eq!(Transition::parse("150ms"), Transition::Ease { ms: 150 });
        assert_eq!(Transition::parse("spring"), Transition::Spring);
        assert_eq!(Transition::parse("bogus"), Transition::None);
        let ease = Transition::Ease { ms: 100 };
        assert_eq!(ease.progress(0.0), 0.0);
        assert!((ease.progress(0.05) - 0.875).abs() < 1e-9);
        assert_eq!(ease.progress(1.0), 1.0);
        // The spring overshoots, then is at rest when its time is up.
        let peak = (0..60).map(|i| Transition::Spring.progress(i as f64 / 100.0)).fold(0.0, f64::max);
        assert!(peak > 1.0 && peak < 1.15, "peak {peak}");
        assert!((Transition::Spring.progress(SPRING_SECS - 1e-6) - 1.0).abs() < 0.01);
    }
}
