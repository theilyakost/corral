use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint,
    FW_SEMIBOLD, FillRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT, ScreenToClient, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW,
    GetWindowTextW, HTTRANSPARENT, HWND_TOP, IDC_ARROW, IDC_SIZEALL, LWA_ALPHA, LoadCursorW,
    MA_NOACTIVATE, RegisterClassW, SWP_NOACTIVATE, SetCursor, SetLayeredWindowAttributes,
    SetWindowLongPtrW, SetWindowPos, WM_CAPTURECHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WNDCLASSW, WS_CHILD,
    WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_VISIBLE,
};
use windows::core::{HSTRING, Result, w};

use crate::config::Group;

pub const HEADER_HEIGHT: i32 = 28;
const HEADER_ALPHA: u8 = 215;
const BODY_ALPHA: u8 = 110;
const HEADER_COLOR: u32 = 0x00201C1A;
const BODY_COLOR: u32 = 0x00141210;
const TEXT_COLOR: u32 = 0x00F0F0F0;

#[derive(Clone, Copy)]
pub struct GroupWindows {
    pub index: usize,
    header: HWND,
    body: HWND,
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
    }
}

impl GroupWindows {
    pub fn create(index: usize, group: &Group, host: HWND, def_view: HWND, instance: HINSTANCE) -> Result<GroupWindows> {
        unsafe {
            let title = HSTRING::from(group.name.as_str());
            let style = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS;
            let ex_style = WS_EX_LAYERED | WS_EX_NOACTIVATE;
            let body = CreateWindowExW(ex_style, w!("CorralBody"), &title, style, 0, 0, 0, 0, Some(host), None, Some(instance), None)?;
            let header = match CreateWindowExW(ex_style, w!("CorralHeader"), &title, style, 0, 0, 0, 0, Some(host), None, Some(instance), None) {
                Ok(header) => header,
                Err(e) => {
                    _ = DestroyWindow(body);
                    return Err(e);
                }
            };
            SetWindowLongPtrW(header, GWLP_USERDATA, index as isize);
            _ = SetLayeredWindowAttributes(header, COLORREF(0), HEADER_ALPHA, LWA_ALPHA);
            _ = SetLayeredWindowAttributes(body, COLORREF(0), BODY_ALPHA, LWA_ALPHA);
            let windows = GroupWindows { index, header, body, host, def_view };
            windows.place(group);
            Ok(windows)
        }
    }

    pub fn place(&self, group: &Group) {
        unsafe {
            let mut origin = POINT { x: group.x, y: group.y };
            _ = ScreenToClient(self.host, &mut origin);
            _ = SetWindowPos(self.header, Some(HWND_TOP), origin.x, origin.y, group.width, HEADER_HEIGHT, SWP_NOACTIVATE);
            _ = SetWindowPos(
                self.body,
                Some(self.def_view),
                origin.x,
                origin.y + HEADER_HEIGHT,
                group.width,
                (group.height - HEADER_HEIGHT).max(0),
                SWP_NOACTIVATE,
            );
        }
    }

    pub fn destroy(&self) {
        unsafe {
            _ = DestroyWindow(self.header);
            _ = DestroyWindow(self.body);
        }
    }
}

unsafe extern "system" fn header_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                paint(hwnd, HEADER_COLOR, true);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_SETCURSOR => {
                SetCursor(LoadCursorW(None, IDC_SIZEALL).ok());
                LRESULT(1)
            }
            WM_LBUTTONDOWN => {
                crate::begin_drag(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize);
                SetCapture(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                crate::drag_to_cursor();
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

unsafe extern "system" fn body_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                paint(hwnd, BODY_COLOR, false);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn paint(hwnd: HWND, color: u32, with_title: bool) {
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
            SetTextColor(hdc, COLORREF(TEXT_COLOR));
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
