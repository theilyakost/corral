use std::cell::Cell;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, GetSystemMetrics, HICON, ICONINFO, SM_CXICON, SM_CXSMICON,
};

const HEADER: [u8; 3] = [0x3D, 0x7E, 0xFF];
const BODY: [u8; 3] = [0x1E, 0x25, 0x30];
const TILE: [u8; 3] = [0xE8, 0xEC, 0xF2];

thread_local! {
    static TRAY: Cell<Option<HICON>> = const { Cell::new(None) };
    static APP: Cell<Option<HICON>> = const { Cell::new(None) };
}

pub fn tray_icon() -> HICON {
    cached(&TRAY, SM_CXSMICON)
}

pub fn app_icon() -> HICON {
    cached(&APP, SM_CXICON)
}

fn cached(slot: &'static std::thread::LocalKey<Cell<Option<HICON>>>, metric: windows::Win32::UI::WindowsAndMessaging::SYSTEM_METRICS_INDEX) -> HICON {
    slot.with(|cell| {
        if let Some(icon) = cell.get() {
            return icon;
        }
        let size = unsafe { GetSystemMetrics(metric) }.max(16);
        let icon = draw(size).unwrap_or_default();
        cell.set(Some(icon));
        icon
    })
}

fn pixel(size: i32, x: i32, y: i32) -> [u8; 4] {
    let f = |v: f32| (v * size as f32).round() as i32;
    let inside = |l: f32, t: f32, r: f32, b: f32| x >= f(l) && x < f(r) && y >= f(t) && y < f(b);
    let [r, g, b] = if inside(0.06, 0.12, 0.94, 0.34) {
        HEADER
    } else if inside(0.06, 0.34, 0.94, 0.88) {
        let tiles = [(0.18, 0.44), (0.56, 0.44), (0.18, 0.66), (0.56, 0.66)];
        if tiles.iter().any(|&(l, t)| inside(l, t, l + 0.26, t + 0.14)) { TILE } else { BODY }
    } else {
        return [0, 0, 0, 0];
    };
    [b, g, r, 0xFF]
}

fn draw(size: i32) -> Option<HICON> {
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let pixels = std::slice::from_raw_parts_mut(bits as *mut [u8; 4], (size * size) as usize);
        for y in 0..size {
            for x in 0..size {
                pixels[(y * size + x) as usize] = pixel(size, x, y);
            }
        }
        let mask = CreateBitmap(size, size, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() });
        _ = DeleteObject(color.into());
        _ = DeleteObject(mask.into());
        icon.ok()
    }
}
