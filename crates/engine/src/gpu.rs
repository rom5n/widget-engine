//! GPU devices shared by every widget in the process: D3D11 -> D2D -> DirectComposition.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::CreateEventW;
use windows::core::*;

pub(crate) struct Gpu {
    pub d2d: ID2D1Factory1,
    pub dwrite: IDWriteFactory,
    pub dcomp: IDCompositionDesktopDevice,
    /// For engine-side GPU work outside a layer draw (wallpaper blur). Its resources work in any layer.
    pub ctx: ID2D1DeviceContext,
    d3d: ID3D11Device4,
    cookie: u32,
    /// Signaled by the driver when the device is removed (TDR, driver update, GPU reset).
    pub lost_event: HANDLE,
}

impl Gpu {
    pub fn new() -> Result<Self> {
        unsafe {
            // Hardware first, WARP (CPU) when no GPU driver is available, e.g. some RDP sessions.
            let d3d = create_d3d(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| create_d3d(D3D_DRIVER_TYPE_WARP))?;
            let d3d: ID3D11Device4 = d3d.cast()?;
            let lost_event = CreateEventW(None, false, false, None)?;
            let cookie = d3d.RegisterDeviceRemovedEvent(lost_event)?;

            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = d2d.CreateDevice(&d3d.cast::<IDXGIDevice>()?)?;
            let dcomp = DCompositionCreateDevice2(&d2d_device)?;
            let ctx = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let dwrite = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Self { d2d, dwrite, dcomp, ctx, d3d, cookie, lost_event })
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            self.d3d.UnregisterDeviceRemoved(self.cookie);
            let _ = CloseHandle(self.lost_event);
        }
    }
}

unsafe fn create_d3d(driver: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    device.ok_or_else(|| Error::from(E_FAIL))
}

/// True when the error means all devices must be recreated.
pub(crate) fn is_device_lost(e: &Error) -> bool {
    [DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, D2DERR_RECREATE_TARGET].contains(&e.code())
}
