use std::ffi::c_void;

use windows::core::BOOL;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;
use windows::Win32::System::Registry::*;
use windows::core::w;

/// System appearance. Widgets are rebuilt when it changes.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub accent: D2D1_COLOR_F,
    /// Text color picked by the user. None: white in dark mode, near-black in light mode.
    pub text: Option<D2D1_COLOR_F>,
}

impl Theme {
    pub fn current() -> Self {
        Self { dark: is_dark(), accent: accent(), text: None }
    }

    /// Main text and artwork color.
    pub fn foreground(&self) -> D2D1_COLOR_F {
        self.text.unwrap_or(if self.dark { rgba(1.0, 1.0, 1.0, 1.0) } else { rgba(0.06, 0.06, 0.06, 1.0) })
    }
}

/// 0xRRGGBB as an opaque color.
pub(crate) fn rgb(v: u32) -> D2D1_COLOR_F {
    let ch = |shift: u32| ((v >> shift) & 0xFF) as f32 / 255.0;
    rgba(ch(16), ch(8), ch(0), 1.0)
}

pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn is_dark() -> bool {
    let mut light = 1u32;
    let mut size = 4u32;
    let _ = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut light as *mut u32 as *mut c_void),
            Some(&mut size),
        )
    };
    light == 0
}

fn accent() -> D2D1_COLOR_F {
    let (mut argb, mut opaque) = (0u32, BOOL(0));
    if unsafe { DwmGetColorizationColor(&mut argb, &mut opaque) }.is_err() {
        return rgba(0.0, 0.47, 0.83, 1.0);
    }
    let ch = |shift: u32| ((argb >> shift) & 0xFF) as f32 / 255.0;
    rgba(ch(16), ch(8), ch(0), 1.0)
}
