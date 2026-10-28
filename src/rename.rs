use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DeleteObject, FW_NORMAL, FW_SEMIBOLD, HFONT,
    OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DLGC_WANTALLKEYS, DestroyWindow, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_CENTER, ES_MULTILINE,
    GetWindowTextLengthW,
    GetWindowTextW, SendMessageW, SetForegroundWindow, WINDOW_STYLE, WM_CHAR, WM_GETDLGCODE, WM_KEYDOWN,
    WM_KILLFOCUS, WM_SETFONT, WS_BORDER, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{HSTRING, w};

const SUBCLASS_ID: usize = 0xC0AB;

#[derive(Clone)]
pub enum Target {
    Group(usize),
    File(String, windows::Win32::UI::Shell::IShellItem),
}

#[derive(Clone, Copy)]
pub enum Look {
    Header,
    Label(f32),
}

thread_local! {
    static ACTIVE: std::cell::RefCell<Option<(HWND, Target, HFONT)>> = const { std::cell::RefCell::new(None) };
    static HEADER_FONT: Cell<Option<HFONT>> = const { Cell::new(None) };
}

fn header_font() -> HFONT {
    HEADER_FONT.with(|cell| {
        *cell.get().get_or_insert_with(|| unsafe {
            CreateFontW(
                17, 0, 0, 0, FW_SEMIBOLD.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe UI"),
            )
        })
    })
}

fn label_font(scale: f32) -> HFONT {
    unsafe {
        CreateFontW(
            -(12.0 * scale).round() as i32, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe UI"),
        )
    }
}

pub fn is_active() -> bool {
    ACTIVE.with(|cell| cell.try_borrow().map_or(true, |a| a.is_some()))
}

pub fn base_name_end(name: &str, is_file: bool) -> usize {
    let total = name.encode_utf16().count();
    if !is_file {
        return total;
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[..dot].encode_utf16().count(),
        _ => total,
    }
}

pub fn start(index: usize, header: RECT, name: &str) {
    start_for(Target::Group(index), header, name, Look::Header);
}

pub fn start_for(target: Target, rect: RECT, name: &str, look: Look) {
    finish(false);
    let is_file = match &target {
        Target::File(_, item) => unsafe {
            use windows::Win32::System::SystemServices::SFGAO_STREAM;
            item.GetAttributes(windows::Win32::System::SystemServices::SFGAO_FLAGS(SFGAO_STREAM.0))
                .is_ok_and(|flags| flags.0 & SFGAO_STREAM.0 != 0)
        },
        Target::Group(_) => false,
    };
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let style = match look {
            Look::Header => WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            Look::Label(_) => WINDOW_STYLE((ES_CENTER | ES_MULTILINE | ES_AUTOVSCROLL) as u32),
        };
        let Ok(edit) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST, w!("EDIT"), &HSTRING::from(name), WS_POPUP | WS_VISIBLE | WS_BORDER | style,
            rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top,
            None, None, Some(instance.into()), None,
        ) else {
            return;
        };
        let (font, owned) = match look {
            Look::Header => (header_font(), HFONT::default()),
            Look::Label(scale) => {
                let font = label_font(scale);
                (font, font)
            }
        };
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        _ = SetWindowSubclass(edit, Some(edit_proc), SUBCLASS_ID, 0);
        ACTIVE.with(|cell| *cell.borrow_mut() = Some((edit, target, owned)));
        _ = SetForegroundWindow(edit);
        _ = SetFocus(Some(edit));
        let end = base_name_end(name, is_file);
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(end as isize)));
    }
}

pub fn finish(commit: bool) {
    let Some((edit, target, font)) = ACTIVE.with(|cell| cell.borrow_mut().take()) else { return };
    unsafe {
        if commit {
            let mut buf = vec![0u16; GetWindowTextLengthW(edit) as usize + 1];
            let len = GetWindowTextW(edit, &mut buf) as usize;
            let name = String::from_utf16_lossy(&buf[..len]).trim().to_string();
            if !name.is_empty() {
                match target {
                    Target::Group(index) => {
                        crate::rename_group(index, &name);
                        crate::settings::refresh();
                    }
                    Target::File(file, item) => crate::rename_file(&file, &item, &name),
                }
            }
        }
        _ = RemoveWindowSubclass(edit, Some(edit_proc), SUBCLASS_ID);
        _ = DestroyWindow(edit);
        if !font.is_invalid() {
            _ = DeleteObject(font.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_skips_the_extension_of_files() {
        assert_eq!(base_name_end("report.final.docx", true), "report.final".len());
        assert_eq!(base_name_end("Отчёт.pdf", true), 5);
        assert_eq!(base_name_end(".gitignore", true), 10);
        assert_eq!(base_name_end("Папка.v2", false), 8);
        assert_eq!(base_name_end("readme", true), 6);
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
