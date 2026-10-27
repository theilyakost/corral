use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, FW_SEMIBOLD, HFONT, OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DLGC_WANTALLKEYS, DestroyWindow, ES_AUTOHSCROLL, GetWindowTextLengthW,
    GetWindowTextW, SendMessageW, SetForegroundWindow, WINDOW_STYLE, WM_CHAR, WM_GETDLGCODE, WM_KEYDOWN,
    WM_KILLFOCUS, WM_SETFONT, WS_BORDER, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{HSTRING, w};

const SUBCLASS_ID: usize = 0xC0AB;

thread_local! {
    static ACTIVE: Cell<Option<(HWND, usize)>> = const { Cell::new(None) };
    static FONT: Cell<Option<HFONT>> = const { Cell::new(None) };
}

fn font() -> HFONT {
    FONT.with(|cell| {
        *cell.get().get_or_insert_with(|| unsafe {
            CreateFontW(
                17, 0, 0, 0, FW_SEMIBOLD.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe UI"),
            )
        })
    })
}

pub fn start(index: usize, header: RECT, name: &str) {
    finish(false);
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let Ok(edit) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST, w!("EDIT"), &HSTRING::from(name),
            WS_POPUP | WS_VISIBLE | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            header.left, header.top, header.right - header.left, header.bottom - header.top,
            None, None, Some(instance.into()), None,
        ) else {
            return;
        };
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font().0 as usize)), Some(LPARAM(1)));
        _ = SetWindowSubclass(edit, Some(edit_proc), SUBCLASS_ID, 0);
        ACTIVE.with(|cell| cell.set(Some((edit, index))));
        _ = SetForegroundWindow(edit);
        _ = SetFocus(Some(edit));
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
}

pub fn finish(commit: bool) {
    let Some((edit, index)) = ACTIVE.with(|cell| cell.take()) else { return };
    unsafe {
        if commit {
            let mut buf = vec![0u16; GetWindowTextLengthW(edit) as usize + 1];
            let len = GetWindowTextW(edit, &mut buf) as usize;
            let name = String::from_utf16_lossy(&buf[..len]).trim().to_string();
            if !name.is_empty() {
                crate::rename_group(index, &name);
                crate::settings::refresh();
            }
        }
        _ = RemoveWindowSubclass(edit, Some(edit_proc), SUBCLASS_ID);
        _ = DestroyWindow(edit);
    }
}

unsafe extern "system" fn edit_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize,
) -> LRESULT {
    match msg {
        WM_GETDLGCODE => LRESULT(DLGC_WANTALLKEYS as isize),
        WM_KEYDOWN if wparam.0 == VK_RETURN.0 as usize => {
            finish(true);
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            finish(false);
            LRESULT(0)
        }
        WM_CHAR if wparam.0 == 0x0D || wparam.0 == 0x1B => LRESULT(0),
        WM_KILLFOCUS => {
            let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
            finish(true);
            result
        }
        _ => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}
