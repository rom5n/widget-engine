//! Engine-drawn widget card: solid, frosted (blurred wallpaper), liquid glass, or clear.
//!
//! System backdrops (Mica, Acrylic) only work on top-level windows. Widgets are children of the
//! desktop, and behind them there are only the wallpaper and the desktop icons. So the engine
//! composes both itself, once per monitor, and each card draws the part that lies under it.

use std::collections::HashMap;
use std::rc::Rc;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;
use windows::core::*;
use windows_numerics::Matrix3x2;

use crate::paint::Shadow;
use crate::settings::{Background, Card};
use crate::theme::{Theme, rgba};

/// The blurred wallpaper is kept at 1/DOWNSAMPLE resolution. The blur hides the difference.
const DOWNSAMPLE: f32 = 4.0;
const BLUR_STD_DEV: f32 = 10.0; // in downsampled pixels, ~40 px on screen
const SATURATION: f32 = 1.5;
/// Liquid glass refracts a sharp backdrop on the CPU: full resolution, barely blurred, so the
/// bending stays visible.
const GLASS_DOWNSAMPLE: f32 = 1.0;
/// Least blur of the glass backdrop, in pixels: just enough to smooth the resampling.
const GLASS_MIN_BLUR: f32 = 0.6;
const GLASS_SATURATION: f32 = 1.15;

/// Blurred wallpaper of one monitor, in monitor space scaled by 1/DOWNSAMPLE.
#[derive(Clone)]
pub(crate) struct Frost {
    bitmap: ID2D1Bitmap1,
    monitor: RECT,
}

/// Lightly blurred wallpaper of one monitor as CPU pixels (premultiplied BGRA), for liquid glass.
#[derive(Clone)]
pub(crate) struct Glass {
    px: Rc<Vec<u8>>,
    w: usize,
    h: usize,
    monitor: RECT,
}

#[derive(Default)]
pub(crate) struct Backdrops {
    /// Keyed by monitor rect. None: no usable wallpaper, frosted falls back to solid.
    cache: HashMap<(i32, i32, i32, i32), Option<Frost>>,
    /// Keyed by monitor rect and blur (1/10 px): one per blur level in use.
    glass: HashMap<(i32, i32, i32, i32, i32), Option<Glass>>,
}

impl Backdrops {
    /// Wallpaper, icons, or displays changed: compose again on next use.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.glass.clear();
    }

    /// Backdrop of the monitor that contains `at`, blurred by `blur_px` (Gaussian sigma, screen
    /// pixels), read back for the glass lens.
    pub fn glass(&mut self, ctx: &ID2D1DeviceContext, at: POINT, blur_px: f32) -> Option<Glass> {
        let (monitor, wallpaper) = wallpaper_at(at)?;
        let blur_px = blur_px.max(GLASS_MIN_BLUR);
        let key = (monitor.left, monitor.top, monitor.right, monitor.bottom, (blur_px * 10.0) as i32);
        if !self.glass.contains_key(&key) {
            // Icons are captured per build and dropped after: 8 MB at 1080p.
            let g = build_glass(ctx, monitor, &wallpaper, capture_icons().as_ref(), blur_px);
            self.glass.insert(key, g.inspect_err(|e| eprintln!("widget engine: liquid glass failed: {e}")).ok());
        }
        self.glass[&key].clone()
    }

    /// Blurred backdrop of the monitor that contains `at` (screen pixels).
    pub fn frost(&mut self, ctx: &ID2D1DeviceContext, at: POINT) -> Option<Frost> {
        let (monitor, wallpaper) = wallpaper_at(at)?;
        let key = (monitor.left, monitor.top, monitor.right, monitor.bottom);
        if !self.cache.contains_key(&key) {
            let f = build_frost(ctx, monitor, &wallpaper, capture_icons().as_ref());
            self.cache.insert(key, f.inspect_err(|e| eprintln!("widget engine: wallpaper blur failed: {e}")).ok());
        }
        self.cache[&key].clone()
    }
}

/// The desktop icon layer as premultiplied BGRA, in screen pixels.
pub(crate) struct Icons {
    px: Vec<u8>,
    rect: RECT,
}

/// Captures the desktop icons. The icon list view renders them over opaque black, so alpha is
/// taken from brightness ("unmultiply"): black turns transparent, bright stays opaque.
// ponytail: the brightness key makes dark icon pixels and label shadows see-through. Exact alpha
// needs a second capture over another background, which Explorer does not offer.
fn capture_icons() -> Option<Icons> {
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    let list = crate::desktop::icon_list()?;
    let rect = crate::desktop::window_rect(list);
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let dc = CreateCompatibleDC(None);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(dib) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        let old = SelectObject(dc, dib.into());
        // PW_RENDERFULLCONTENT: copy what DWM composed. The plain WM_PRINT path draws nothing here.
        let ok = PrintWindow(list, dc, PRINT_WINDOW_FLAGS(2)).as_bool();
        let mut px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
        SelectObject(dc, old);
        let _ = DeleteObject(dib.into());
        let _ = DeleteDC(dc);
        if !ok {
            return None;
        }
        unmultiply(&mut px);
        Some(Icons { px, rect })
    }
}

/// BGRA over black to premultiplied BGRA: alpha = brightest channel.
fn unmultiply(px: &mut [u8]) {
    for p in px.chunks_exact_mut(4) {
        p[3] = p[0].max(p[1]).max(p[2]);
    }
}

struct Wallpaper {
    path: String,
    position: DESKTOP_WALLPAPER_POSITION,
    background: COLORREF,
}

fn wallpaper_at(at: POINT) -> Option<(RECT, Wallpaper)> {
    unsafe {
        let dw: IDesktopWallpaper = CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL).ok()?;
        let position = dw.GetPosition().unwrap_or(DWPOS_FILL);
        let background = dw.GetBackgroundColor().unwrap_or(COLORREF(0));
        for i in 0..dw.GetMonitorDevicePathCount().ok()? {
            let Ok(id) = dw.GetMonitorDevicePathAt(i) else { continue };
            let rect = dw.GetMonitorRECT(PCWSTR(id.0));
            let path = dw.GetWallpaper(PCWSTR(id.0)).ok().map(|p| {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as _));
                s
            });
            CoTaskMemFree(Some(id.0 as _));
            let Ok(rect) = rect else { continue };
            if at.x >= rect.left && at.x < rect.right && at.y >= rect.top && at.y < rect.bottom {
                return Some((rect, Wallpaper { path: path.unwrap_or_default(), position, background }));
            }
        }
        None
    }
}

fn build_frost(ctx: &ID2D1DeviceContext, monitor: RECT, wp: &Wallpaper, icons: Option<&Icons>) -> Result<Frost> {
    let frosted = blurred(ctx, monitor, wp, icons, DOWNSAMPLE, BLUR_STD_DEV, SATURATION)?;
    Ok(Frost { bitmap: frosted, monitor })
}

fn build_glass(ctx: &ID2D1DeviceContext, monitor: RECT, wp: &Wallpaper, icons: Option<&Icons>, blur_px: f32) -> Result<Glass> {
    let bitmap = blurred(ctx, monitor, wp, icons, GLASS_DOWNSAMPLE, blur_px, GLASS_SATURATION)?;
    unsafe {
        let size = bitmap.GetPixelSize();
        let readable = ctx.CreateBitmap(
            size,
            None,
            0,
            &D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            },
        )?;
        readable.CopyFromBitmap(None, &bitmap, None)?;
        let map = readable.Map(D2D1_MAP_OPTIONS_READ)?;
        let (w, h) = (size.width as usize, size.height as usize);
        let mut px = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            px.extend_from_slice(std::slice::from_raw_parts(map.bits.add(y * map.pitch as usize), w * 4));
        }
        readable.Unmap()?;
        ctx.GetDevice()?.ClearResources(0);
        Ok(Glass { px: Rc::new(px), w, h, monitor })
    }
}

/// The monitor's wallpaper and icons at 1/`downsample` size, blurred and saturated, as a GPU bitmap.
fn blurred(
    ctx: &ID2D1DeviceContext,
    monitor: RECT,
    wp: &Wallpaper,
    icons: Option<&Icons>,
    downsample: f32,
    blur_px: f32,
    saturation: f32,
) -> Result<ID2D1Bitmap1> {
    let w = ((monitor.right - monitor.left) as f32 / downsample).ceil().max(1.0);
    let h = ((monitor.bottom - monitor.top) as f32 / downsample).ceil().max(1.0);
    unsafe {
        let composed = target_bitmap(ctx, w as u32, h as u32)?;
        let frosted = target_bitmap(ctx, w as u32, h as u32)?;

        // 1. Wallpaper laid out like Explorer does, at reduced size.
        ctx.SetTarget(&composed);
        ctx.SetDpi(96.0, 96.0);
        ctx.SetTransform(&Matrix3x2::identity());
        ctx.BeginDraw();
        let bg = wp.background.0;
        ctx.Clear(Some(&rgba(
            (bg & 0xFF) as f32 / 255.0,
            ((bg >> 8) & 0xFF) as f32 / 255.0,
            ((bg >> 16) & 0xFF) as f32 / 255.0,
            1.0,
        )));
        if !wp.path.is_empty() {
            if let Ok(image) = load_image(ctx, &wp.path, downsample) {
                // Already decoded at 1/DOWNSAMPLE size.
                let size = image.GetPixelSize();
                let (iw, ih) = (size.width as f32, size.height as f32);
                let tile = wp.position == DWPOS_TILE;
                let dest = if tile { [0.0, 0.0, iw, ih] } else { layout(wp.position, iw, ih, w, h) };
                let extend = if tile { D2D1_EXTEND_MODE_WRAP } else { D2D1_EXTEND_MODE_CLAMP };
                let brush = ctx.CreateBitmapBrush(
                    &image,
                    Some(&D2D1_BITMAP_BRUSH_PROPERTIES1 {
                        extendModeX: extend,
                        extendModeY: extend,
                        interpolationMode: D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                    }),
                    None,
                )?;
                let (sx, sy) = ((dest[2] / size.width as f32), (dest[3] / size.height as f32));
                brush.SetTransform(&Matrix3x2 { M11: sx, M12: 0.0, M21: 0.0, M22: sy, M31: dest[0], M32: dest[1] });
                let fill = if tile { [0.0, 0.0, w, h] } else { dest };
                ctx.FillRectangle(&rect(fill[0], fill[1], fill[0] + fill[2], fill[1] + fill[3]), &brush);
            }
        }
        // 2. The icons on top, as Explorer shows them.
        if let Some(icons) = icons {
            let r = icons.rect;
            let (iw, ih) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);
            let bitmap = ctx.CreateBitmap(
                D2D_SIZE_U { width: iw, height: ih },
                Some(icons.px.as_ptr().cast()),
                iw * 4,
                &D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: 96.0,
                    dpiY: 96.0,
                    ..Default::default()
                },
            )?;
            let at = |v: i32, origin: i32| (v - origin) as f32 / downsample;
            let dest = rect(at(r.left, monitor.left), at(r.top, monitor.top), at(r.right, monitor.left), at(r.bottom, monitor.top));
            ctx.DrawBitmap(&bitmap, Some(&dest), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
        }
        ctx.EndDraw(None, None)?;

        // 3. Blur and saturate into the cached bitmap.
        let blur = ctx.CreateEffect(&CLSID_D2D1GaussianBlur)?;
        blur.SetInput(0, &composed, true);
        set_f32(&blur, D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32, blur_px)?;
        set_u32(&blur, D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32, D2D1_BORDER_MODE_HARD.0 as u32)?;
        let saturate = ctx.CreateEffect(&CLSID_D2D1Saturation)?;
        saturate.SetInput(0, &blur.GetOutput()?, true);
        set_f32(&saturate, D2D1_SATURATION_PROP_SATURATION.0 as u32, saturation)?;

        ctx.SetTarget(&frosted);
        ctx.BeginDraw();
        ctx.DrawImage(&saturate.GetOutput()?, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_COPY);
        ctx.EndDraw(None, None)?;
        ctx.SetTarget(None);
        // Drop the effect intermediates and the decoded wallpaper. Only `frosted` stays.
        ctx.GetDevice()?.ClearResources(0);
        Ok(frosted)
    }
}

/// Destination rect [x, y, w, h] of an image of size (iw, ih) on a screen of size (sw, sh).
// ponytail: Span is treated as Fill per monitor; exact span math needs the virtual-screen rect.
pub(crate) fn layout(pos: DESKTOP_WALLPAPER_POSITION, iw: f32, ih: f32, sw: f32, sh: f32) -> [f32; 4] {
    let (w, h) = match pos {
        DWPOS_STRETCH => (sw, sh),
        DWPOS_CENTER => (iw, ih),
        DWPOS_FIT => {
            let k = (sw / iw).min(sh / ih);
            (iw * k, ih * k)
        }
        _ => {
            let k = (sw / iw).max(sh / ih);
            (iw * k, ih * k)
        }
    };
    [(sw - w) / 2.0, (sh - h) / 2.0, w, h]
}

unsafe fn target_bitmap(ctx: &ID2D1DeviceContext, w: u32, h: u32) -> Result<ID2D1Bitmap1> {
    unsafe {
        ctx.CreateBitmap(
            D2D_SIZE_U { width: w, height: h },
            None,
            0,
            &D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                ..Default::default()
            },
        )
    }
}

/// Decodes an image file into a premultiplied BGRA bitmap, scaled down by `downsample`.
pub(crate) unsafe fn load_image(ctx: &ID2D1DeviceContext, path: &str, downsample: f32) -> Result<ID2D1Bitmap1> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = wic.CreateDecoderFromFilename(&HSTRING::from(path), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
        decode(ctx, &wic, &decoder, downsample)
    }
}

/// Decodes an image in memory (PNG, JPEG, ...) into a premultiplied BGRA bitmap.
pub(crate) unsafe fn load_image_bytes(ctx: &ID2D1DeviceContext, bytes: &[u8]) -> Result<ID2D1Bitmap1> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = windows::Win32::UI::Shell::SHCreateMemStream(Some(bytes)).ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_OUTOFMEMORY))?;
        let decoder = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        decode(ctx, &wic, &decoder, 1.0)
    }
}

unsafe fn decode(ctx: &ID2D1DeviceContext, wic: &IWICImagingFactory, decoder: &IWICBitmapDecoder, downsample: f32) -> Result<ID2D1Bitmap1> {
    unsafe {
        let frame = decoder.GetFrame(0)?;
        // Decode straight to the target size: a 4K wallpaper never reaches the GPU in full.
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h)?;
        let scaler = wic.CreateBitmapScaler()?;
        let (sw, sh) = (((w as f32 / downsample) as u32).max(1), ((h as f32 / downsample) as u32).max(1));
        scaler.Initialize(&frame, sw, sh, WICBitmapInterpolationModeFant)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeMedianCut,
        )?;
        ctx.CreateBitmapFromWicBitmap(&converter, None)
    }
}

unsafe fn set_f32(effect: &ID2D1Effect, prop: u32, v: f32) -> Result<()> {
    unsafe { effect.SetValue(prop, D2D1_PROPERTY_TYPE_FLOAT, &v.to_le_bytes()) }
}

unsafe fn set_u32(effect: &ID2D1Effect, prop: u32, v: u32) -> Result<()> {
    unsafe { effect.SetValue(prop, D2D1_PROPERTY_TYPE_ENUM, &v.to_le_bytes()) }
}

/// Inputs for drawing one card.
pub(crate) struct CardParams {
    pub card: Card,
    pub theme: Theme,
    pub opacity: f32,
    pub tint: Option<u32>,
    /// Liquid glass tint alpha, see `Resolved::veil`.
    pub veil: f32,
    /// Widget top-left in screen pixels.
    pub origin: POINT,
    /// Pixels per DIP.
    pub scale: f32,
    pub size: (f32, f32),
    pub frost: Option<Frost>,
    pub glass: Option<Glass>,
}

/// Shadow under a card when the user turns it on: a soft ambient one and a tight contact one.
/// Glass gets only a faint wide one: a dark contact shadow reads as smudges around the corners.
pub(crate) fn card_shadows(background: Background) -> Vec<Shadow> {
    if background == Background::Glass {
        return vec![Shadow { dx: 0.0, dy: 10.0, blur: 24.0, color: rgba(0.0, 0.0, 0.0, 0.06) }];
    }
    vec![
        Shadow { dx: 0.0, dy: 4.0, blur: 14.0, color: rgba(0.0, 0.0, 0.0, 0.32) },
        Shadow { dx: 0.0, dy: 1.0, blur: 2.0, color: rgba(0.0, 0.0, 0.0, 0.22) },
    ]
}

/// Draws the card into a layer context (DIPs, layer-relative).
pub(crate) unsafe fn draw_card(ctx: &ID2D1DeviceContext, d2d: &ID2D1Factory1, p: &CardParams) -> Result<()> {
    let (w, h) = p.size;
    let dark = p.theme.dark;
    // Circular corners for every material, the same as liquid glass (see `draw_glass`), so a
    // widget keeps its outline when the user switches the background.
    let r = p.card.radius.min(w / 2.0).min(h / 2.0);
    let shape = unsafe { d2d.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: rect(0.5, 0.5, w - 0.5, h - 0.5), radiusX: r, radiusY: r })? };
    let base = match p.tint {
        Some(rgb) => rgba(
            ((rgb >> 16) & 0xFF) as f32 / 255.0,
            ((rgb >> 8) & 0xFF) as f32 / 255.0,
            (rgb & 0xFF) as f32 / 255.0,
            1.0,
        ),
        None if dark => rgba(0.12, 0.12, 0.13, 1.0),
        // Widgets on macOS are pure white in light mode.
        None => rgba(1.0, 1.0, 1.0, 1.0),
    };
    let with_alpha = |a: f32| D2D1_COLOR_F { a, ..base };
    unsafe {
        let frost = p.frost.as_ref().filter(|_| p.card.background == Background::Frosted);
        match (p.card.background, frost) {
            (Background::Frosted, Some(f)) => {
                let brush = ctx.CreateBitmapBrush(
                    &f.bitmap,
                    Some(&D2D1_BITMAP_BRUSH_PROPERTIES1 {
                        extendModeX: D2D1_EXTEND_MODE_CLAMP,
                        extendModeY: D2D1_EXTEND_MODE_CLAMP,
                        interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR,
                    }),
                    None,
                )?;
                // Bitmap pixel b is screen pixel monitor.left + b * DOWNSAMPLE.
                // Layer DIP d is screen pixel origin + d * scale.
                let k = DOWNSAMPLE / p.scale;
                brush.SetTransform(&Matrix3x2 {
                    M11: k,
                    M12: 0.0,
                    M21: 0.0,
                    M22: k,
                    M31: (f.monitor.left - p.origin.x) as f32 / p.scale,
                    M32: (f.monitor.top - p.origin.y) as f32 / p.scale,
                });
                brush.SetOpacity(p.opacity);
                ctx.FillGeometry(&shape, &brush, None);
                let tint = if dark { 0.38 } else { 0.42 };
                ctx.FillGeometry(&shape, &ctx.CreateSolidColorBrush(&with_alpha(tint * p.opacity), None)?, None);
            }
            (Background::Glass, _) if p.glass.is_some() => {
                draw_glass(ctx, p, p.glass.as_ref().unwrap())?;
                // Apple's "regular" glass: a milky veil in light mode; dark mode dims with pure
                // black, since gray turns the backdrop muddy.
                let veil = match p.tint {
                    Some(_) => with_alpha(p.veil),
                    None if dark => rgba(0.0, 0.0, 0.0, p.veil),
                    None => rgba(1.0, 1.0, 1.0, p.veil),
                };
                let veil = D2D1_COLOR_F { a: veil.a * p.opacity, ..veil };
                // Same circular corners as the glass itself (see `draw_glass`), edge to edge.
                let round = d2d.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: rect(0.0, 0.0, w, h), radiusX: r, radiusY: r })?;
                ctx.FillGeometry(&round, &ctx.CreateSolidColorBrush(&veil, None)?, None);
            }
            (Background::Clear, _) => {
                ctx.FillGeometry(&shape, &ctx.CreateSolidColorBrush(&with_alpha(0.12 * p.opacity), None)?, None);
            }
            // Solid, or frosted without a wallpaper.
            _ => {
                ctx.FillGeometry(&shape, &ctx.CreateSolidColorBrush(&with_alpha(p.opacity), None)?, None);
            }
        }
        if p.card.border && p.card.background != Background::Glass {
            let edge = if dark { rgba(1.0, 1.0, 1.0, 0.10) } else { rgba(1.0, 1.0, 1.0, 0.55) };
            ctx.DrawGeometry(&shape, &ctx.CreateSolidColorBrush(&edge, None)?, 1.0, None);
        }
    }
    Ok(())
}

/// Height of the glass across the bezel: 0 at the edge, 1 where the flat top starts.
/// Convex squircle, the profile Apple's Liquid Glass appears to use: a soft flat-to-curve
/// transition, so the refraction has no hard inner edge.
fn glass_profile(x: f32) -> f32 {
    (1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(4)).powf(0.25)
}

/// Slope of the glass surface at `x` across the bezel, for a glass `thickness` in bezel widths.
fn glass_slope(x: f32, thickness: f32) -> f32 {
    let e = 1e-3;
    let x = x.clamp(e, 1.0 - e);
    thickness * (glass_profile(x + e) - glass_profile(x - e)) / (2.0 * e)
}

/// How far a vertical ray moves sideways after refracting into the glass at `x` across the
/// bezel (0 edge, 1 flat), in bezel widths. Snell's law with n = 1.5, one refraction, then the
/// ray travels down through the glass to the backdrop.
pub(crate) fn glass_displacement(x: f32, thickness: f32) -> f32 {
    if x >= 1.0 {
        return 0.0;
    }
    let incidence = glass_slope(x, thickness).atan();
    let refracted = (incidence.sin() / 1.5).asin();
    thickness * glass_profile(x) * (incidence - refracted).tan()
}

/// Liquid glass: a convex glass slab over the backdrop (wallpaper and icons). Each card pixel
/// shows the backdrop refracted through the curved bezel, so content near the edge is pulled
/// inward and magnified, with a little color dispersion, plus a specular rim lit from the top
/// left. The flat middle stays sharp. Method after kube.io, "Liquid Glass in the Browser:
/// Refraction with CSS and SVG". Rendered on the CPU at screen resolution, only when the card
/// is redrawn (rebuild, drag, icons change).
// ponytail: CPU lens, a few ms for a 400x200 px card. Move to a D2D pixel shader if glass animates.
unsafe fn draw_glass(ctx: &ID2D1DeviceContext, p: &CardParams, g: &Glass) -> Result<()> {
    let (w, h) = p.size;
    let s = p.scale;
    let (pw, ph) = ((w * s).ceil() as usize, (h * s).ceil() as usize);
    let r = p.card.radius.min(w / 2.0).min(h / 2.0);
    // Bezel width in DIPs, and glass thickness in bezel widths.
    // Apple's bezel is a narrow band: the middle shows the backdrop undistorted.
    let bezel = (w.min(h) * 0.07).clamp(8.0, 14.0);
    let thickness = 2.2;
    // Displacement along one radius, precomputed: it is the same all around the bezel.
    const STEPS: usize = 128;
    let table: Vec<f32> = (0..=STEPS).map(|i| glass_displacement(i as f32 / STEPS as f32, thickness) * bezel).collect();
    // Signed distance to the rounded rect edge, negative inside (DIPs). Circular corners: the
    // glass veil in `draw_card` must use the same shape, or it sticks out at the corners.
    let sdf = |x: f32, y: f32| {
        let (qx, qy) = ((x - w / 2.0).abs() - (w / 2.0 - r), (y - h / 2.0).abs() - (h / 2.0 - r));
        (qx.max(0.0).hypot(qy.max(0.0))) + qx.max(qy).min(0.0) - r
    };
    let sample = |sx: f32, sy: f32, c: usize| -> f32 {
        // Screen pixel to backdrop pixel, bilinear, clamped at the monitor edge.
        let bx = ((sx - g.monitor.left as f32) / GLASS_DOWNSAMPLE - 0.5).clamp(0.0, (g.w - 1) as f32);
        let by = ((sy - g.monitor.top as f32) / GLASS_DOWNSAMPLE - 0.5).clamp(0.0, (g.h - 1) as f32);
        let (x0, y0) = (bx as usize, by as usize);
        let (x1, y1) = ((x0 + 1).min(g.w - 1), (y0 + 1).min(g.h - 1));
        let (fx, fy) = (bx - x0 as f32, by - y0 as f32);
        let at = |x: usize, y: usize| g.px[(y * g.w + x) * 4 + c] as f32;
        let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        top * (1.0 - fy) + bottom * fy
    };
    let mut px = vec![0u8; pw * ph * 4];
    // Light from the top left. Highlights show on the lit edge and, weaker, on the opposite one.
    let light = (-0.707f32, -0.707f32);
    for y in 0..ph {
        for x in 0..pw {
            let (dx, dy) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let d = sdf(dx, dy);
            let coverage = (-d * s + 0.5).clamp(0.0, 1.0);
            if coverage == 0.0 {
                continue;
            }
            let depth = -d;
            let (mut ox, mut oy, mut rim, mut shade) = (0.0, 0.0, 0.0, 0.0);
            if depth < bezel {
                // Outward normal from the distance field.
                let e = 0.5;
                let (nx, ny) = (sdf(dx + e, dy) - sdf(dx - e, dy), sdf(dx, dy + e) - sdf(dx, dy - e));
                let len = nx.hypot(ny).max(1e-6);
                let (nx, ny) = (nx / len, ny / len);
                let t = (depth / bezel).max(0.0);
                let f = t * STEPS as f32;
                let i = (f as usize).min(STEPS - 1);
                let disp = table[i] + (table[i + 1] - table[i]) * (f - i as f32);
                // Convex glass bends rays toward the middle: sample inward.
                (ox, oy) = (-nx * disp, -ny * disp);
                // Specular: a crisp 1 DIP line all around, brightest at the corner facing the
                // light and the one opposite, plus a soft glow on the lit side. A faint dark line
                // just inside the rim on the shaded side separates the glass from the backdrop.
                let facing = -(nx * light.0 + ny * light.1);
                let lit = facing * facing * if facing > 0.0 { 1.0 } else { 0.8 };
                rim = (-depth / 0.6).exp() * (0.18 + 0.6 * lit) + (-depth / 4.0).exp() * 0.06 * facing.max(0.0);
                shade = 0.06 * (-(depth - 1.2).abs() / 0.5).exp() * (-facing).max(0.0);
            }
            let (sx, sy) = (p.origin.x as f32 + dx * s, p.origin.y as f32 + dy * s);
            // Glass stays opaque: the backdrop is already in it. Transparency thins the tint instead.
            let a = coverage;
            let o = (y * pw + x) * 4;
            // Dispersion: blue (channel 0 in BGRA) bends a little more than red.
            for (ch, k) in [(0usize, 1.06f32), (1, 1.03), (2, 1.0)] {
                let c = sample(sx + ox * k * s, sy + oy * k * s, ch);
                let c = c * (1.0 - shade);
                let v = c + (255.0 - c) * rim;
                px[o + ch] = (v * a).round().clamp(0.0, 255.0) as u8;
            }
            px[o + 3] = (255.0 * a).round() as u8;
        }
    }
    unsafe {
        let bitmap = ctx.CreateBitmap(
            D2D_SIZE_U { width: pw as u32, height: ph as u32 },
            Some(px.as_ptr().cast()),
            (pw * 4) as u32,
            &D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                // One bitmap pixel per screen pixel.
                dpiX: 96.0 * s,
                dpiY: 96.0 * s,
                ..Default::default()
            },
        )?;
        ctx.DrawImage(&bitmap, None, None, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
    }
    Ok(())
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: [f32; 4], b: [f32; 4]) {
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01), "{a:?} != {b:?}");
    }

    #[test]
    fn glass_bends_most_at_the_edge() {
        let d: Vec<f32> = [0.05, 0.2, 0.5, 1.0].iter().map(|&x| glass_displacement(x, 1.2)).collect();
        assert!(d[0] > d[1] && d[1] > d[2] && d[2] > d[3], "{d:?}");
        // The flat top does not bend.
        assert!(d[3].abs() < 1e-3);
        // Black turns transparent, white stays opaque, colors keep their brightest channel.
        let mut px = [0, 0, 0, 255, 255, 255, 255, 255, 10, 200, 40, 255];
        unmultiply(&mut px);
        assert_eq!([px[3], px[7], px[11]], [0, 255, 200]);
    }

    #[test]
    fn wallpaper_layout() {
        // 3000x2000 image on a 1920x1080 screen.
        approx(layout(DWPOS_STRETCH, 3000.0, 2000.0, 1920.0, 1080.0), [0.0, 0.0, 1920.0, 1080.0]);
        // Fill covers: scale by width, crop top and bottom.
        approx(layout(DWPOS_FILL, 3000.0, 2000.0, 1920.0, 1080.0), [0.0, -100.0, 1920.0, 1280.0]);
        // Fit contains: scale by height, bars left and right.
        approx(layout(DWPOS_FIT, 3000.0, 2000.0, 1920.0, 1080.0), [150.0, 0.0, 1620.0, 1080.0]);
        approx(layout(DWPOS_CENTER, 1000.0, 500.0, 1920.0, 1080.0), [460.0, 290.0, 1000.0, 500.0]);
    }
}
