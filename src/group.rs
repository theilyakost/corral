use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint,
    FW_SEMIBOLD, FillRect, InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, ScreenToClient, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::System::SystemServices::MK_LBUTTON;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetCapture, ReleaseCapture, SetCapture, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect, GetCursorPos,
    GetWindowLongPtrW, GetWindowTextW, HTCLIENT, HTTRANSPARENT, HWND_TOP, IDC_ARROW, IDC_SIZEALL, IDC_SIZENS,
    IDC_SIZENWSE, IDC_SIZEWE, LWA_ALPHA, LoadCursorW,
    MA_NOACTIVATE, RegisterClassW, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOZORDER, SetCursor, SetLayeredWindowAttributes,
    IsWindowVisible, SetWindowTextW, ShowWindow,
    LWA_COLORKEY, SetWindowLongPtrW, SetWindowPos, WM_CAPTURECHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WNDCLASSW, WS_CHILD,
    WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TRANSPARENT, WS_VISIBLE,
};
use windows::core::{HSTRING, Result, w};

use std::cell::Cell;

use crate::config::{Appearance, Group, parse_color};

pub const HEADER_HEIGHT: i32 = 28;
#[derive(Clone, Copy)]
struct Style {
    header: u32,
    body: u32,
    title: u32,
    header_alpha: u8,
    body_alpha: u8,
}

thread_local! {
    static STYLE: Cell<Style> = const {
        Cell::new(Style { header: 0x00201C1A, body: 0x00141210, title: 0x00F0F0F0, header_alpha: 215, body_alpha: 110 })
    };
}

pub fn set_style(appearance: &Appearance) {
    let fallback = STYLE.with(Cell::get);
    let color = |text: &str, default: u32| parse_color(text).unwrap_or(default);
    STYLE.with(|cell| {
        cell.set(Style {
            header: color(&appearance.header_color, fallback.header),
            body: color(&appearance.body_color, fallback.body),
            title: color(&appearance.title_color, fallback.title),
            header_alpha: appearance.header_opacity,
            body_alpha: appearance.body_opacity,
        })
    });
}

fn current_style() -> Style {
    STYLE.with(Cell::get)
}

pub fn body_color() -> u32 {
    current_style().body
}
const GRIP: i32 = 8;

#[derive(Clone, Copy)]
pub struct GroupWindows {
    pub index: usize,
    header: HWND,
    pub body: HWND,
    glass: Option<HWND>,
    host: HWND,
    def_view: HWND,
}

pub fn register_classes(instance: HINSTANCE) {
    unsafe {
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        for (name, proc_) in [
            (w!("CorralHeader"), header_proc as unsafe extern "system" fn(_, _, _, _) -> _),
            (w!("CorralBody"), body_proc),
        ] {
            let class = WNDCLASSW {
                lpfnWndProc: Some(proc_),
                hInstance: instance,
                lpszClassName: name,
                hCursor: cursor,
                ..Default::default()
            };
            RegisterClassW(&class);
        }
        let shelf = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(crate::shelf::shelf_proc),
            hInstance: instance,
            lpszClassName: w!("CorralShelf"),
            hCursor: cursor,
            ..Default::default()
        };
        RegisterClassW(&shelf);
        let glass = WNDCLASSW {
            lpfnWndProc: Some(crate::shelf::glass_proc),
            hInstance: instance,
            lpszClassName: w!("CorralGlass"),
            ..Default::default()
        };
        RegisterClassW(&glass);
    }
}

impl GroupWindows {
    pub fn create(
        index: usize, group: &Group, host: HWND, def_view: HWND, instance: HINSTANCE, owned: bool,
    ) -> Result<GroupWindows> {
        unsafe {
            let title = HSTRING::from(group.name.as_str());
            let style = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS;
            let ex_style = WS_EX_LAYERED | WS_EX_NOACTIVATE;
            let class = if owned { w!("CorralShelf") } else { w!("CorralBody") };
            let body = CreateWindowExW(ex_style, class, &title, style, 0, 0, 0, 0, Some(host), None, Some(instance), None)?;
            let header = match CreateWindowExW(ex_style, w!("CorralHeader"), &title, style, 0, 0, 0, 0, Some(host), None, Some(instance), None) {
                Ok(header) => header,
                Err(e) => {
                    _ = DestroyWindow(body);
                    return Err(e);
                }
            };
            SetWindowLongPtrW(header, GWLP_USERDATA, index as isize);
            SetWindowLongPtrW(body, GWLP_USERDATA, index as isize);
            crate::drop::register(header, index, true);
            crate::drop::register(body, index, false);
            _ = SetLayeredWindowAttributes(header, COLORREF(0), current_style().header_alpha, LWA_ALPHA);
            _ = SetLayeredWindowAttributes(body, COLORREF(0), current_style().body_alpha, LWA_ALPHA);
            let glass = if owned {
                let ex_style = ex_style | WS_EX_TRANSPARENT;
                let glass = CreateWindowExW(ex_style, w!("CorralGlass"), &title, style, 0, 0, 0, 0, Some(host), None, Some(instance), None);
                let glass = match glass {
                    Ok(glass) => glass,
                    Err(e) => {
                        _ = DestroyWindow(body);
                        _ = DestroyWindow(header);
                        return Err(e);
                    }
                };
                SetWindowLongPtrW(glass, GWLP_USERDATA, body.0 as isize);
                _ = SetLayeredWindowAttributes(glass, crate::shelf::GLASS_KEY, 0, LWA_COLORKEY);
                crate::shelf::register(body, glass);
                Some(glass)
            } else {
                None
            };
            let windows = GroupWindows { index, header, body, glass, host, def_view };
            windows.place(group);
            if owned {
                windows.set_collapsed(group, group.collapsed);
            }
            Ok(windows)
        }
    }

    pub fn place(&self, group: &Group) {
        unsafe {
            let mut origin = POINT { x: group.x, y: group.y };
            _ = ScreenToClient(self.host, &mut origin);
            _ = SetWindowPos(self.header, Some(HWND_TOP), origin.x, origin.y, group.width, HEADER_HEIGHT, SWP_NOACTIVATE);
            let (width, height) = (group.width, (group.height - HEADER_HEIGHT).max(0));
            if self.glass.is_some() {
                crate::shelf::set_extent(self.body, width, height);
            }
            if let Some(glass) = self.glass {
                _ = SetWindowPos(glass, Some(self.header), origin.x, origin.y + HEADER_HEIGHT, width, height, SWP_NOACTIVATE);
            }
            let below = self.glass.unwrap_or(self.def_view);
            _ = SetWindowPos(
                self.body,
                Some(below),
                origin.x,
                origin.y + HEADER_HEIGHT,
                width,
                height,
                SWP_NOACTIVATE,
            );
        }
    }

    pub fn set_open(&self, open: bool) {
        let command = if open { SW_SHOWNA } else { SW_HIDE };
        unsafe {
            _ = ShowWindow(self.body, command);
            if let Some(glass) = self.glass {
                _ = ShowWindow(glass, command);
            }
        }
    }

    pub fn restyle(&self) {
        let style = current_style();
        unsafe {
            _ = SetLayeredWindowAttributes(self.header, COLORREF(0), style.header_alpha, LWA_ALPHA);
            _ = SetLayeredWindowAttributes(self.body, COLORREF(0), style.body_alpha, LWA_ALPHA);
            _ = InvalidateRect(Some(self.header), None, false);
            _ = InvalidateRect(Some(self.body), None, false);
        }
    }

    pub fn set_title(&self, text: &str) {
        unsafe {
            _ = SetWindowTextW(self.header, &HSTRING::from(text));
            _ = InvalidateRect(Some(self.header), None, false);
        }
    }

    pub fn is_open(&self) -> bool {
        unsafe { IsWindowVisible(self.body).as_bool() }
    }

    pub fn reveal(&self, group: &Group, fraction: f32) {
        let full = (group.height - HEADER_HEIGHT).max(0);
        let height = (full as f32 * fraction.clamp(0.0, 1.0)).round() as i32;
        unsafe {
            let mut origin = POINT { x: group.x, y: group.y };
            _ = ScreenToClient(self.host, &mut origin);
            let flags = SWP_NOACTIVATE | SWP_NOZORDER;
            let (x, y) = (origin.x, origin.y + HEADER_HEIGHT);
            if let Some(glass) = self.glass {
                _ = SetWindowPos(glass, None, x, y, group.width, height, flags);
            }
            _ = SetWindowPos(self.body, None, x, y, group.width, height, flags);
        }
    }

    pub fn set_marker(&self, group: &Group, collapsed: bool) {
        let marker = if collapsed { "▸" } else { "▾" };
        unsafe {
            _ = SetWindowTextW(self.header, &HSTRING::from(format!("{marker}  {}", group.name)));
            _ = InvalidateRect(Some(self.header), None, false);
        }
    }

    pub fn set_collapsed(&self, group: &Group, collapsed: bool) {
        self.set_open(!collapsed);
        self.set_marker(group, collapsed);
    }

    pub fn destroy(&self) {
        crate::drop::revoke(self.header);
        crate::drop::revoke(self.body);
        unsafe {
            _ = DestroyWindow(self.header);
            _ = DestroyWindow(self.body);
            if let Some(glass) = self.glass {
                _ = DestroyWindow(glass);
            }
        }
    }
}

unsafe extern "system" fn header_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                paint(hwnd, current_style().header, true);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_SETCURSOR => {
                SetCursor(LoadCursorW(None, IDC_SIZEALL).ok());
                LRESULT(1)
            }
            WM_LBUTTONDOWN => {
                SetCapture(hwnd);
                crate::begin_drag(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if GetCapture() == hwnd {
                    if released(wparam) {
                        _ = ReleaseCapture();
                    } else {
                        crate::drag_to_cursor();
                    }
                } else {
                    crate::hover_header(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                _ = ReleaseCapture();
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                crate::end_drag();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn grip_at(hwnd: HWND, point: POINT) -> Option<(bool, bool)> {
    let mut rect = RECT::default();
    unsafe { _ = GetClientRect(hwnd, &mut rect) };
    let right = point.x >= rect.right - GRIP;
    let bottom = point.y >= rect.bottom - GRIP;
    (right || bottom).then_some((right, bottom))
}

fn grip_under_cursor(hwnd: HWND) -> Option<(bool, bool)> {
    let mut point = POINT::default();
    unsafe {
        _ = GetCursorPos(&mut point);
        _ = ScreenToClient(hwnd, &mut point);
    }
    grip_at(hwnd, point)
}

fn released(wparam: WPARAM) -> bool {
    wparam.0 & MK_LBUTTON.0 as usize == 0 || unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } >= 0
}

pub unsafe fn grip_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    unsafe {
        let captured = GetCapture() == hwnd;
        match msg {
            WM_SETCURSOR => {
                let cursor = match grip_under_cursor(hwnd)? {
                    (true, true) => IDC_SIZENWSE,
                    (true, false) => IDC_SIZEWE,
                    _ => IDC_SIZENS,
                };
                SetCursor(LoadCursorW(None, cursor).ok());
                Some(LRESULT(1))
            }
            WM_LBUTTONDOWN => {
                let point = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
                let (right, bottom) = grip_at(hwnd, point)?;
                SetCapture(hwnd);
                crate::begin_resize(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize, right, bottom);
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE if captured => {
                if released(wparam) {
                    _ = ReleaseCapture();
                } else {
                    crate::drag_to_cursor();
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP if captured => {
                _ = ReleaseCapture();
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                crate::end_drag();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

unsafe extern "system" fn body_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if let Some(result) = grip_message(hwnd, msg, wparam, lparam) {
            return result;
        }
        match msg {
            WM_PAINT => {
                paint(hwnd, current_style().body, false);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_NCHITTEST if grip_under_cursor(hwnd).is_some() => LRESULT(HTCLIENT as isize),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

pub unsafe fn paint(hwnd: HWND, color: u32, with_title: bool) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rect = RECT::default();
        _ = GetClientRect(hwnd, &mut rect);

        let brush = CreateSolidBrush(COLORREF(color));
        FillRect(hdc, &rect, brush);
        _ = DeleteObject(brush.into());

        if with_title {
            let mut buf = [0u16; 256];
            let len = GetWindowTextW(hwnd, &mut buf) as usize;
            let font = CreateFontW(
                17, 0, 0, 0, FW_SEMIBOLD.0 as i32, 0, 0, 0,
                DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0,
                w!("Segoe UI"),
            );
            let old = SelectObject(hdc, font.into());
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(current_style().title));
            let mut text_rect = RECT { left: rect.left + 10, right: rect.right - 10, ..rect };
            DrawTextW(
                hdc,
                &mut buf[..len],
                &mut text_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            SelectObject(hdc, old);
            _ = DeleteObject(font.into());
        }

        _ = EndPaint(hwnd, &ps);
    }
}
