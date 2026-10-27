use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory,
    ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_EMERGENCY_BREAK, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, ClientToScreen, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, EndPaint, HBITMAP, HDC, HGDIOBJ, InvalidateRect, PAINTSTRUCT, SRCCOPY,
    SelectObject,
};
use windows::Win32::Graphics::Imaging::IWICBitmap;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{DragDetect, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::IShellItem;
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, HTTRANSPARENT, MA_NOACTIVATE, WM_DESTROY,
    WM_ERASEBKGND, WM_NCHITTEST, WM_DRAWITEM, WM_INITMENUPOPUP, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MEASUREITEM,
    WM_MENUCHAR, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_RBUTTONUP,
};
use windows::core::{Result, w};

use crate::shell::{self, MenuChoice};

pub const GLASS_KEY: COLORREF = COLORREF(0x0002_0103);
const KEY_COLOR: D2D1_COLOR_F = D2D1_COLOR_F { r: 3.0 / 255.0, g: 1.0 / 255.0, b: 2.0 / 255.0, a: 1.0 };
const SHADOW_COLOR: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.75 };

#[derive(Clone, Copy)]
struct Palette {
    text: D2D1_COLOR_F,
    shadow: bool,
    hover: D2D1_COLOR_F,
    selected: D2D1_COLOR_F,
}

fn rgb(color: u32) -> (f32, f32, f32) {
    let channel = |shift: u32| ((color >> shift) & 0xFF) as f32 / 255.0;
    (channel(0), channel(8), channel(16))
}

fn luminance((r, g, b): (f32, f32, f32)) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn palette() -> Palette {
    let (r, g, b) = rgb(crate::group::label_color());
    let body = rgb(crate::group::body_color());
    let toward = if luminance(body) > 0.5 { 0.0 } else { 1.0 };
    let mix = |amount: f32| D2D1_COLOR_F {
        r: body.0 + (toward - body.0) * amount,
        g: body.1 + (toward - body.1) * amount,
        b: body.2 + (toward - body.2) * amount,
        a: 1.0,
    };
    Palette {
        text: D2D1_COLOR_F { r, g, b, a: 1.0 },
        shadow: luminance((r, g, b)) > 0.5,
        hover: mix(0.14),
        selected: mix(0.26),
    }
}

const CELL_WIDTH: f32 = 76.0;
const CELL_HEIGHT: f32 = 96.0;
const ICON_SIZE: f32 = 48.0;
const ICON_TOP: f32 = 6.0;
const LABEL_GAP: f32 = 4.0;
const LABEL_LINES: f32 = 2.0;
const FONT_SIZE: f32 = 12.0;
const PADDING: f32 = 4.0;

pub struct Entry {
    name: String,
    label: Vec<u16>,
    item: IShellItem,
    icon: Option<(i32, Option<IWICBitmap>)>,
    bitmap: Option<(u64, ID2D1Bitmap)>,
}

struct Frame {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    width: i32,
    height: i32,
}

impl Frame {
    fn new(width: i32, height: i32) -> Option<Frame> {
        unsafe {
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let dc = CreateCompatibleDC(None);
            let mut bits = std::ptr::null_mut();
            let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
                _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, bitmap.into());
            Some(Frame { dc, bitmap, old, width, height })
        }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            _ = DeleteObject(self.bitmap.into());
            _ = DeleteDC(self.dc);
        }
    }
}

#[derive(Default)]
struct Shelf {
    glass: HWND,
    entries: Vec<Entry>,
    hover: Option<usize>,
    selected: Option<usize>,
    scroll: f32,
    tracking: bool,
    extent: (i32, i32),
    frame: Option<Frame>,
    dirty: bool,
}

struct Graphics {
    target: ID2D1DCRenderTarget,
    write: IDWriteFactory,
    formats: Vec<(u32, IDWriteTextFormat)>,
    generation: u64,
}

struct Metrics {
    scale: f32,
    columns: usize,
    cell_width: f32,
    cell_height: f32,
}

thread_local! {
    static SHELVES: RefCell<HashMap<isize, Shelf>> = RefCell::new(HashMap::new());
    static GRAPHICS: RefCell<Option<Graphics>> = const { RefCell::new(None) };
    static SPACING: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

fn with_shelf<R>(hwnd: HWND, f: impl FnOnce(&mut Shelf) -> R) -> Option<R> {
    SHELVES.with(|cell| cell.try_borrow_mut().ok()?.get_mut(&(hwnd.0 as isize)).map(f))
}

pub fn set_spacing(spacing: POINT) {
    SPACING.with(|cell| cell.set(Some((spacing.x, spacing.y))));
}

pub fn register(body: HWND, glass: HWND) {
    SHELVES.with(|cell| _ = cell.borrow_mut().insert(body.0 as isize, Shelf { glass, dirty: true, ..Default::default() }));
}

pub fn set_extent(body: HWND, width: i32, height: i32) {
    let changed = with_shelf(body, |shelf| {
        let changed = shelf.extent != (width, height);
        if changed {
            shelf.extent = (width, height);
            shelf.dirty = true;
        }
        changed
    });
    if changed == Some(true) {
        render(body);
    }
}

pub fn set_items(hwnd: HWND, items: Vec<(String, IShellItem)>) {
    let Some(known) = with_shelf(hwnd, |shelf| shelf.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>())
    else {
        return;
    };
    let fresh: Vec<Entry> = items
        .into_iter()
        .map(|(name, item)| {
            let label = if known.contains(&name) { Vec::new() } else { shell::label(&item).encode_utf16().collect() };
            Entry { name, label, item, icon: None, bitmap: None }
        })
        .collect();
    with_shelf(hwnd, |shelf| {
        let selected = shelf.selected.and_then(|i| shelf.entries.get(i)).map(|e| e.name.clone());
        let mut old: HashMap<String, Entry> = shelf.entries.drain(..).map(|e| (e.name.clone(), e)).collect();
        shelf.entries = fresh.into_iter().map(|e| old.remove(&e.name).unwrap_or(e)).collect();
        shelf.selected = selected.and_then(|name| shelf.entries.iter().position(|e| e.name == name));
        shelf.hover = None;
    });
    render(hwnd);
}

fn load_icons(body: HWND, pixels: i32) {
    let missing: Vec<(usize, IShellItem)> = with_shelf(body, |shelf| {
        let stale = |e: &&Entry| e.icon.as_ref().is_none_or(|(size, _)| *size != pixels);
        shelf.entries.iter().enumerate().filter(|(_, e)| stale(e)).map(|(i, e)| (i, e.item.clone())).collect()
    })
    .unwrap_or_default();
    let loaded: Vec<_> = missing.into_iter().map(|(i, item)| (i, shell::icon(&item, pixels))).collect();
    with_shelf(body, |shelf| {
        for (i, icon) in loaded {
            if let Some(entry) = shelf.entries.get_mut(i) {
                entry.icon = Some((pixels, icon));
                entry.bitmap = None;
            }
        }
    });
}

fn metrics(hwnd: HWND, width: f32) -> Metrics {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
    let (step_x, cell_height) = SPACING
        .with(|cell| cell.get())
        .map_or((CELL_WIDTH * scale, CELL_HEIGHT * scale), |(x, y)| (x as f32, y as f32));
    let columns = ((width / step_x).round() as usize).max(1);
    let cell_width = width.max(step_x) / columns as f32;
    Metrics { scale, columns, cell_width, cell_height }
}

impl Metrics {
    fn cell(&self, index: usize, scroll: f32) -> D2D_RECT_F {
        let (col, row) = ((index % self.columns) as f32, (index / self.columns) as f32);
        let (w, h) = (self.cell_width, self.cell_height);
        let left = col * w;
        let top = PADDING * self.scale + row * h - scroll;
        D2D_RECT_F { left, top, right: left + w, bottom: top + h }
    }

    fn content_height(&self, count: usize) -> f32 {
        let rows = count.div_ceil(self.columns) as f32;
        rows * self.cell_height + 2.0 * PADDING * self.scale
    }

    fn icon_pixels(&self) -> i32 {
        (ICON_SIZE * self.scale).round() as i32
    }
}

fn client_size(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe { _ = GetClientRect(hwnd, &mut rect) };
    (rect.right - rect.left, rect.bottom - rect.top)
}

fn hit(hwnd: HWND, lparam: LPARAM) -> Option<usize> {
    let (x, y) = ((lparam.0 & 0xFFFF) as i16 as f32, ((lparam.0 >> 16) & 0xFFFF) as i16 as f32);
    let (width, _) = client_size(hwnd);
    let metrics = metrics(hwnd, width as f32);
    with_shelf(hwnd, |shelf| {
        (0..shelf.entries.len()).find(|&i| {
            let r = metrics.cell(i, shelf.scroll);
            x >= r.left && x < r.right && y >= r.top && y < r.bottom
        })
    })
    .flatten()
}

fn graphics() -> Result<()> {
    GRAPHICS.with(|cell| {
        if cell.borrow().is_some() {
            return Ok(());
        }
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let target = factory.CreateDCRenderTarget(&properties)?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let generation = GENERATION.with(|g| {
                g.set(g.get() + 1);
                g.get()
            });
            *cell.borrow_mut() = Some(Graphics { target, write, formats: Vec::new(), generation });
            Ok(())
        }
    })
}

impl Graphics {
    fn format(&mut self, scale: f32) -> Result<IDWriteTextFormat> {
        let key = (scale * 96.0).round() as u32;
        if let Some((_, format)) = self.formats.iter().find(|(k, _)| *k == key) {
            return Ok(format.clone());
        }
        unsafe {
            let format = self.write.CreateTextFormat(
                w!("Segoe UI"), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL, FONT_SIZE * scale, w!(""),
            )?;
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            _ = format.SetWordWrapping(DWRITE_WORD_WRAPPING_EMERGENCY_BREAK);
            let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, ..Default::default() };
            let sign = self.write.CreateEllipsisTrimmingSign(&format)?;
            format.SetTrimming(&trimming, &sign)?;
            self.formats.push((key, format.clone()));
            Ok(format)
        }
    }
}

pub fn render(body: HWND) {
    if let Some(glass) = with_shelf(body, |shelf| {
        shelf.dirty = true;
        shelf.glass
    }) {
        unsafe { _ = InvalidateRect(Some(glass), None, false) };
    }
}

fn paint_glass(glass: HWND, hdc: HDC) {
    let body = HWND(unsafe { GetWindowLongPtrW(glass, GWLP_USERDATA) } as _);
    let ready = match graphics().and_then(|_| prepare(body, glass)) {
        Ok(ready) => ready,
        Err(_) => {
            GRAPHICS.with(|cell| *cell.borrow_mut() = None);
            false
        }
    };
    let (width, height) = client_size(glass);
    let blitted = with_shelf(body, |shelf| match &shelf.frame {
        Some(frame) if ready => unsafe {
            _ = BitBlt(hdc, 0, 0, width.min(frame.width), height.min(frame.height), Some(frame.dc), 0, 0, SRCCOPY);
            true
        },
        _ => false,
    });
    if !ready || blitted.is_none() {
        unsafe { _ = InvalidateRect(Some(glass), None, false) };
    }
}

fn prepare(body: HWND, glass: HWND) -> Result<bool> {
    let Some((dirty, extent)) = with_shelf(body, |shelf| {
        let fits = shelf.frame.as_ref().is_some_and(|f| (f.width, f.height) == shelf.extent);
        (shelf.dirty || !fits, shelf.extent)
    }) else {
        return Ok(false);
    };
    if !dirty {
        return Ok(true);
    }
    let (width, height) = if extent.0 > 0 && extent.1 > 0 { extent } else { client_size(glass) };
    if width <= 0 || height <= 0 {
        return Ok(true);
    }
    let metrics = metrics(body, width as f32);
    load_icons(body, metrics.icon_pixels());
    GRAPHICS.with(|cell| {
        let mut graphics = cell.borrow_mut();
        let graphics = graphics.as_mut().ok_or_else(windows::core::Error::empty)?;
        let format = graphics.format(metrics.scale)?;
        let generation = graphics.generation;
        let target = &graphics.target;
        let drawn = with_shelf(body, |shelf| -> Result<()> {
            if shelf.frame.as_ref().is_none_or(|f| (f.width, f.height) != (width, height)) {
                shelf.frame = Frame::new(width, height);
            }
            let Some(frame) = &shelf.frame else { return Err(windows::core::Error::empty()) };
            unsafe {
                target.BindDC(frame.dc, &RECT { left: 0, top: 0, right: width, bottom: height })?;
                target.BeginDraw();
                target.Clear(Some(&KEY_COLOR));
                let palette = palette();
                let brush = target.CreateSolidColorBrush(&palette.text, None)?;
                let max_scroll = (metrics.content_height(shelf.entries.len()) - height as f32).max(0.0);
                shelf.scroll = shelf.scroll.clamp(0.0, max_scroll);
                for index in 0..shelf.entries.len() {
                    let cell = metrics.cell(index, shelf.scroll);
                    if cell.bottom < 0.0 || cell.top > height as f32 {
                        continue;
                    }
                    let highlight = if shelf.selected == Some(index) {
                        Some(palette.selected)
                    } else if shelf.hover == Some(index) {
                        Some(palette.hover)
                    } else {
                        None
                    };
                    let entry = &mut shelf.entries[index];
                    let bitmap = entry_bitmap(target, entry, generation);
                    draw_entry(target, &brush, &format, &metrics, &palette, entry, bitmap.as_ref(), cell, highlight);
                }
                target.EndDraw(None, None)?;
                crate::group::frame_body(frame.dc, width, height, GLASS_KEY.0);
            }
            shelf.dirty = false;
            Ok(())
        });
        match drawn {
            Some(result) => result.map(|_| true),
            None => Ok(false),
        }
    })
}

fn entry_bitmap(target: &ID2D1DCRenderTarget, entry: &mut Entry, generation: u64) -> Option<ID2D1Bitmap> {
    if let Some((built, bitmap)) = &entry.bitmap {
        if *built == generation {
            return Some(bitmap.clone());
        }
    }
    let Some((_, Some(icon))) = &entry.icon else { return None };
    let bitmap = unsafe { target.CreateBitmapFromWicBitmap(icon, None).ok()? };
    entry.bitmap = Some((generation, bitmap.clone()));
    Some(bitmap)
}

fn draw_entry(
    target: &ID2D1DCRenderTarget,
    brush: &ID2D1SolidColorBrush,
    format: &IDWriteTextFormat,
    metrics: &Metrics,
    palette: &Palette,
    entry: &Entry,
    bitmap: Option<&ID2D1Bitmap>,
    cell: D2D_RECT_F,
    highlight: Option<D2D1_COLOR_F>,
) {
    let s = metrics.scale;
    unsafe {
        if let Some(color) = highlight {
            let inset = 2.0 * s;
            let rect = D2D_RECT_F { left: cell.left + inset, top: cell.top + inset, right: cell.right - inset, bottom: cell.bottom - inset };
            brush.SetColor(&color);
            target.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect, radiusX: 4.0 * s, radiusY: 4.0 * s }, brush);
        }

        let pixels = metrics.icon_pixels();
        let icon_top = cell.top + ICON_TOP * s;
        if let Some(bitmap) = bitmap {
            let size = bitmap.GetPixelSize();
            let (w, h) = (size.width as f32, size.height as f32);
            let x = (cell.left + cell.right - w) / 2.0;
            let y = icon_top + (pixels as f32 - h) / 2.0;
            let dest = D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h };
            target.DrawBitmap(bitmap, Some(&dest), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
        }

        let top = icon_top + pixels as f32 + LABEL_GAP * s;
        let line = FONT_SIZE * s * 1.34;
        let text = D2D_RECT_F { left: cell.left + 3.0 * s, top, right: cell.right - 3.0 * s, bottom: top + line * LABEL_LINES };
        let shadow = D2D_RECT_F { left: text.left + s, top: text.top + s, right: text.right + s, bottom: text.bottom + s };
        if palette.shadow {
            brush.SetColor(&SHADOW_COLOR);
            target.DrawText(&entry.label, format, &shadow, brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
        }
        brush.SetColor(&palette.text);
        target.DrawText(&entry.label, format, &text, brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
    }
}

fn set_state(hwnd: HWND, f: impl FnOnce(&mut Shelf) -> bool) {
    if with_shelf(hwnd, f) == Some(true) {
        render(hwnd);
    }
}

fn entry(hwnd: HWND, index: usize) -> Option<(String, IShellItem)> {
    with_shelf(hwnd, |shelf| shelf.entries.get(index).map(|e| (e.name.clone(), e.item.clone()))).flatten()
}

pub unsafe extern "system" fn shelf_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if let Some(result) = crate::group::grip_message(hwnd, msg, wparam, lparam) {
            return result;
        }
        if matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
            if let Some(result) = shell::forward_menu_message(msg, wparam, lparam) {
                return result;
            }
        }
        match msg {
            WM_PAINT => {
                crate::group::paint(hwnd, crate::group::body_color(), false, crate::group::Edges::Nothing);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_MOUSEMOVE => {
                let index = hit(hwnd, lparam);
                let track = with_shelf(hwnd, |shelf| !std::mem::replace(&mut shelf.tracking, true)) == Some(true);
                if track {
                    let mut event = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    _ = TrackMouseEvent(&mut event);
                }
                set_state(hwnd, |shelf| std::mem::replace(&mut shelf.hover, index) != index);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                set_state(hwnd, |shelf| {
                    shelf.tracking = false;
                    shelf.hover.take().is_some()
                });
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let index = hit(hwnd, lparam);
                set_state(hwnd, |shelf| std::mem::replace(&mut shelf.selected, index) != index);
                let mut at = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
                _ = ClientToScreen(hwnd, &mut at);
                if let Some((name, item)) = index.and_then(|i| entry(hwnd, i)) {
                    if DragDetect(hwnd, at).as_bool() {
                        crate::drag_member(hwnd, GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize, &name, &item);
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                if let Some((_, item)) = hit(hwnd, lparam).and_then(|i| entry(hwnd, i)) {
                    shell::open(&item);
                }
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                let Some(index) = hit(hwnd, lparam) else { return LRESULT(0) };
                set_state(hwnd, |shelf| std::mem::replace(&mut shelf.selected, Some(index)) != Some(index));
                let Some((name, item)) = entry(hwnd, index) else { return LRESULT(0) };
                let mut at = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
                _ = ClientToScreen(hwnd, &mut at);
                if let Some(MenuChoice::Extra) = shell::context_menu(hwnd, &item, "Убрать из группы", at) {
                    crate::take_out(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize, &name);
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as f32 / 120.0;
                let step = metrics(hwnd, client_size(hwnd).0 as f32).cell_height;
                set_state(hwnd, |shelf| {
                    let before = shelf.scroll;
                    shelf.scroll = (shelf.scroll - delta * step).max(0.0);
                    shelf.scroll != before
                });
                LRESULT(0)
            }
            WM_DESTROY => {
                SHELVES.with(|cell| _ = cell.borrow_mut().remove(&(hwnd.0 as isize)));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

pub unsafe extern "system" fn glass_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                paint_glass(hwnd, hdc);
                _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
