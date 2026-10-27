use std::sync::Once;

use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, GetDC, ReleaseDC};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetCapture, ReleaseCapture, SetCapture, VK_ESCAPE, VK_LBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetSystemMetrics, MSG,
    MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, RegisterClassW, SM_SWAPBUTTON, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, ShowWindow, TranslateMessage, ULW_ALPHA,
    UpdateLayeredWindow, WINDOW_STYLE, WM_MOUSEFIRST, WM_MOUSELAST, WM_QUIT, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WindowFromPoint,
};
use windows::core::w;

const IMAGE_ALPHA: u8 = 220;
const FRAME_MS: u32 = 15;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Outcome {
    Group(usize),
    Desktop,
    Handoff,
    Cancel,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Target {
    Group(usize),
    Desktop,
    Elsewhere,
}

pub fn outcome(target: Target, released: bool) -> Option<Outcome> {
    match (target, released) {
        (Target::Elsewhere, _) => Some(Outcome::Handoff),
        (Target::Group(index), true) => Some(Outcome::Group(index)),
        (Target::Desktop, true) => Some(Outcome::Desktop),
        _ => None,
    }
}

fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(image_proc),
            hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
            lpszClassName: w!("CorralDragImage"),
            ..Default::default()
        };
        RegisterClassW(&class);
    });
}

unsafe extern "system" fn image_proc(
    hwnd: HWND, msg: u32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn button_down() -> bool {
    let button = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 { VK_RBUTTON } else { VK_LBUTTON };
    unsafe { GetAsyncKeyState(button.0 as i32) < 0 }
}

fn image_origin(cursor: POINT, width: i32, height: i32) -> POINT {
    POINT { x: cursor.x - width / 2, y: cursor.y - height / 3 }
}

pub fn run(shelf: HWND, entry: usize, classify: impl Fn(POINT, HWND) -> Target) -> Outcome {
    register();
    let frame = crate::shelf::drag_image(shelf, entry);
    let mut cursor = POINT::default();
    unsafe { _ = GetCursorPos(&mut cursor) };
    let image = frame.as_ref().and_then(|frame| unsafe {
        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        let origin = image_origin(cursor, frame.width, frame.height);
        let window = CreateWindowExW(
            ex, w!("CorralDragImage"), None, WS_POPUP | WINDOW_STYLE::default(),
            origin.x, origin.y, frame.width, frame.height, None, None, None, None,
        )
        .ok()?;
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: IMAGE_ALPHA,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let screen = GetDC(None);
        _ = UpdateLayeredWindow(
            window, Some(screen), Some(&origin), Some(&SIZE { cx: frame.width, cy: frame.height }),
            Some(frame.dc), Some(&POINT::default()), COLORREF(0), Some(&blend), ULW_ALPHA,
        );
        ReleaseDC(None, screen);
        _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        Some(window)
    });
    let size = frame.as_ref().map_or((0, 0), |f| (f.width, f.height));

    unsafe { SetCapture(shelf) };
    let result = loop {
        if unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) } < 0 || unsafe { GetCapture() } != shelf {
            break Outcome::Cancel;
        }
        let released = !button_down();
        unsafe { _ = GetCursorPos(&mut cursor) };
        if let Some(window) = image {
            let origin = image_origin(cursor, size.0, size.1);
            unsafe {
                _ = SetWindowPos(window, None, origin.x, origin.y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        let under = unsafe { WindowFromPoint(cursor) };
        let target = classify(cursor, under);
        if let Some(outcome) = outcome(target, released) {
            break outcome;
        }
        if released {
            break Outcome::Cancel;
        }
        if let Target::Group(index) = target {
            crate::hover_header(index);
        }
        if !pump(shelf) {
            break Outcome::Cancel;
        }
    };

    unsafe {
        if GetCapture() == shelf {
            _ = ReleaseCapture();
        }
        if let Some(window) = image {
            _ = DestroyWindow(window);
        }
    }
    result
}

fn pump(shelf: HWND) -> bool {
    unsafe {
        MsgWaitForMultipleObjects(None, false, FRAME_MS, QS_ALLINPUT);
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(msg.wParam.0 as i32);
                return false;
            }
            if msg.hwnd == shelf && (WM_MOUSEFIRST..=WM_MOUSELAST).contains(&msg.message) {
                continue;
            }
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_windows_hand_over_to_the_shell() {
        assert_eq!(outcome(Target::Elsewhere, false), Some(Outcome::Handoff));
        assert_eq!(outcome(Target::Elsewhere, true), Some(Outcome::Handoff));
    }

    #[test]
    fn drops_happen_only_on_release() {
        assert_eq!(outcome(Target::Desktop, false), None);
        assert_eq!(outcome(Target::Group(2), false), None);
        assert_eq!(outcome(Target::Desktop, true), Some(Outcome::Desktop));
        assert_eq!(outcome(Target::Group(2), true), Some(Outcome::Group(2)));
    }
}
