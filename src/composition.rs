use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HMODULE, HWND, POINT};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
    D2D1CreateFactory, ID2D1DeviceContext, ID2D1Factory1,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionRectangleClip, IDCompositionSurface,
    IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGISurface};
use windows::core::{Interface, Result};

pub struct Device {
    pub context: ID2D1DeviceContext,
    dcomp: IDCompositionDevice,
    pub generation: u64,
}

thread_local! {
    static DEVICE: RefCell<Option<Device>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

fn create() -> Result<Device> {
    unsafe {
        let mut d3d: Option<ID3D11Device> = None;
        D3D11CreateDevice(
            None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None,
            D3D11_SDK_VERSION, Some(&mut d3d), None, None,
        )?;
        let d3d = d3d.ok_or_else(windows::core::Error::empty)?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let context = factory.CreateDevice(&dxgi)?.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
        context.SetDpi(96.0, 96.0);
        context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
        let generation = GENERATION.with(|g| {
            g.set(g.get() + 1);
            g.get()
        });
        Ok(Device { context, dcomp, generation })
    }
}

pub fn with_device<R>(f: impl FnOnce(&Device) -> Result<R>) -> Result<R> {
    DEVICE.with(|cell| {
        let mut device = cell.try_borrow_mut().map_err(|_| windows::core::Error::empty())?;
        if device.is_none() {
            *device = Some(create()?);
        }
        let result = f(device.as_ref().ok_or_else(windows::core::Error::empty)?);
        if result.is_err() {
            *device = None;
        }
        result
    })
}

pub struct Panel {
    target: IDCompositionTarget,
    visual: IDCompositionVisual,
    clip: IDCompositionRectangleClip,
    surface: Option<(IDCompositionSurface, i32, i32)>,
    generation: u64,
}

impl Panel {
    pub fn new(hwnd: HWND) -> Result<Panel> {
        with_device(|device| unsafe {
            let target = device.dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = device.dcomp.CreateVisual()?;
            let clip = device.dcomp.CreateRectangleClip()?;
            visual.SetClip(&clip)?;
            target.SetRoot(&visual)?;
            Ok(Panel { target, visual, clip, surface: None, generation: device.generation })
        })
    }

    pub fn is_current(&self) -> bool {
        DEVICE.with(|cell| cell.try_borrow().ok().and_then(|d| d.as_ref().map(|d| d.generation)) == Some(self.generation))
    }

    pub fn draw(&mut self, width: i32, height: i32, paint: impl FnOnce(&Device, f32, f32) -> Result<()>) -> Result<()> {
        if width <= 0 || height <= 0 {
            return Ok(());
        }
        with_device(|device| unsafe {
            if self.surface.as_ref().is_none_or(|(_, w, h)| (*w, *h) != (width, height)) {
                let surface = device.dcomp.CreateSurface(
                    width as u32, height as u32, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED,
                )?;
                self.visual.SetContent(&surface)?;
                self.surface = Some((surface, width, height));
            }
            let Some((surface, _, _)) = &self.surface else { return Err(windows::core::Error::empty()) };
            let mut offset = POINT::default();
            let dxgi: IDXGISurface = surface.BeginDraw(None, &mut offset)?;
            let properties = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            let bitmap = device.context.CreateBitmapFromDxgiSurface(&dxgi, Some(&properties))?;
            device.context.SetTarget(&bitmap);
            device.context.BeginDraw();
            device.context.Clear(Some(&D2D1_COLOR_F::default()));
            let painted = paint(device, offset.x as f32, offset.y as f32);
            let ended = device.context.EndDraw(None, None);
            device.context.SetTarget(None);
            surface.EndDraw()?;
            painted?;
            ended?;
            device.dcomp.Commit()
        })
    }

    pub fn clip(&self, width: i32, height: i32, radius: f32) -> Result<()> {
        with_device(|device| unsafe {
            self.clip.SetLeft2(0.0)?;
            self.clip.SetTop2(0.0)?;
            self.clip.SetRight2(width as f32)?;
            self.clip.SetBottom2(height as f32)?;
            for set in [
                IDCompositionRectangleClip::SetTopLeftRadiusX2,
                IDCompositionRectangleClip::SetTopLeftRadiusY2,
                IDCompositionRectangleClip::SetTopRightRadiusX2,
                IDCompositionRectangleClip::SetTopRightRadiusY2,
                IDCompositionRectangleClip::SetBottomLeftRadiusX2,
                IDCompositionRectangleClip::SetBottomLeftRadiusY2,
                IDCompositionRectangleClip::SetBottomRightRadiusX2,
                IDCompositionRectangleClip::SetBottomRightRadiusY2,
            ] {
                set(&self.clip, radius)?;
            }
            let _ = &self.target;
            device.dcomp.Commit()
        })
    }
}
