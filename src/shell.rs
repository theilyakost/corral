use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits,
    GetObjectW, HBITMAP,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmap, IWICImagingFactory,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::{
    FOF_ALLOWUNDO, FOFX_EARLYFAILURE, FileOperation, IFileOperation, SIGDN_FILESYSPATH, BHID_SFUIObject, CMF_CANRENAME, CMF_NORMAL, GCS_VERBA, GCS_VERBW, IShellFolder, SHBindToParent, SHCreateItemWithParent,
    SHGDN_FOREDITING,
    SHGDN_FORPARSING, SHGDN_INFOLDER, SHGDNF, StrRetToStrW, CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFOEX, IContextMenu,
    IContextMenu2, IContextMenu3, IShellItem, IShellItemImageFactory, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW,
    SHGetIDListFromObject, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetMenuStringW, HMENU, MF_BYCOMMAND, MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL,
    SetForegroundWindow,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
};
use windows::Win32::UI::Shell::Common::{ITEMIDLIST, STRRET};
use windows::core::{HSTRING, Interface, PCSTR, PCWSTR, PSTR, PWSTR, Result};

const FIRST_COMMAND: u32 = 1;
const LAST_COMMAND: u32 = 0x7EFF;
const EXTRA_COMMAND: u32 = 0x7F00;
const CMIC_MASK_UNICODE: u32 = 0x4000;

thread_local! {
    static WIC: RefCell<Option<IWICImagingFactory>> = const { RefCell::new(None) };
    static ACTIVE_MENU: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
}

pub fn label(item: &IShellItem) -> String {
    unsafe {
        let Ok(name) = item.GetDisplayName(SIGDN_NORMALDISPLAY) else { return String::new() };
        let text = name.to_string().unwrap_or_default();
        CoTaskMemFree(Some(name.0 as _));
        text
    }
}

pub fn normalize(pixels: &mut [u8]) {
    if pixels.chunks_exact(4).all(|p| p[3] == 0) {
        pixels.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        return;
    }
    let premultiplied = pixels.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
    if !premultiplied {
        for p in pixels.chunks_exact_mut(4) {
            let alpha = p[3] as u32;
            for channel in &mut p[..3] {
                *channel = ((*channel as u32 * alpha + 127) / 255) as u8;
            }
        }
    }
}

fn pixels_of(bitmap: HBITMAP) -> Option<(i32, i32, Vec<u8>)> {
    unsafe {
        let mut info = BITMAP::default();
        if GetObjectW(bitmap.into(), size_of::<BITMAP>() as i32, Some(&mut info as *mut _ as *mut _)) == 0 {
            return None;
        }
        let (width, height) = (info.bmWidth, info.bmHeight.abs());
        if width <= 0 || height <= 0 {
            return None;
        }
        let mut header = BITMAPINFO {
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
        let mut buffer = vec![0u8; (width * height * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, bitmap, 0, height as u32, Some(buffer.as_mut_ptr() as *mut _), &mut header, DIB_RGB_COLORS);
        _ = DeleteDC(dc);
        (lines == height).then_some((width, height, buffer))
    }
}

const PREVIEWED: &[&str] = &[
    "png", "jpg", "jpeg", "jfif", "gif", "bmp", "webp", "tif", "tiff", "heic", "heif", "avif", "ico", "svg", "raw",
    "cr2", "nef", "dng", "psd", "mp4", "mkv", "avi", "mov", "wmv", "webm", "m4v", "mpg", "mpeg", "3gp",
];

pub fn shows_preview(name: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PREVIEWED.iter().any(|p| p.eq_ignore_ascii_case(e)))
}

fn parsing_name(item: &IShellItem) -> Option<String> {
    unsafe {
        let name = item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).ok()?;
        let text = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as _));
        text
    }
}

pub fn item_at(path: &std::path::Path) -> Option<IShellItem> {
    unsafe { windows::Win32::UI::Shell::SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok() }
}

pub fn icon(item: &IShellItem, size: i32) -> Option<IWICBitmap> {
    unsafe {
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let preview = parsing_name(item).is_some_and(|n| shows_preview(&n));
        let flags = if preview { SIIGBF_RESIZETOFIT } else { SIIGBF_RESIZETOFIT | SIIGBF_ICONONLY };
        let bitmap = factory.GetImage(SIZE { cx: size, cy: size }, flags).ok()?;
        let pixels = pixels_of(bitmap);
        _ = DeleteObject(bitmap.into());
        let (width, height, mut buffer) = pixels?;
        normalize(&mut buffer);
        let wic = WIC.with(|cell| {
            let mut cell = cell.borrow_mut();
            if cell.is_none() {
                *cell = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok();
            }
            cell.clone()
        })?;
        wic.CreateBitmapFromMemory(width as u32, height as u32, &GUID_WICPixelFormat32bppPBGRA, (width * 4) as u32, &buffer)
            .ok()
    }
}

pub fn open(item: &IShellItem) {
    unsafe {
        let Ok(pidl) = SHGetIDListFromObject(item) else { return };
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_INVOKEIDLIST,
            lpIDList: pidl as _,
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        _ = ShellExecuteExW(&mut info);
        CoTaskMemFree(Some(pidl as _));
    }
}

pub enum MenuChoice {
    Extra,
    Rename,
    Done,
}

fn verb(menu: &IContextMenu, id: usize) -> Option<String> {
    let mut wide = [0u16; 64];
    let unicode = unsafe {
        menu.GetCommandString(id, GCS_VERBW, None, PSTR(wide.as_mut_ptr() as *mut u8), wide.len() as u32)
    };
    if unicode.is_ok() {
        let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        if len > 0 {
            return Some(String::from_utf16_lossy(&wide[..len]));
        }
    }
    let mut narrow = [0u8; 64];
    unsafe { menu.GetCommandString(id, GCS_VERBA, None, PSTR(narrow.as_mut_ptr()), narrow.len() as u32).ok()? };
    let len = narrow.iter().position(|&c| c == 0).unwrap_or(narrow.len());
    (len > 0).then(|| String::from_utf8_lossy(&narrow[..len]).into_owned())
}

fn item_text(popup: HMENU, command: u32) -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetMenuStringW(popup, command, Some(&mut buf), MF_BYCOMMAND) } as usize;
    String::from_utf16_lossy(&buf[..len.min(buf.len())]).replace('&', "")
}

pub fn is_rename(verb: Option<&str>, text: &str) -> bool {
    match verb {
        Some(verb) if !verb.is_empty() => verb.eq_ignore_ascii_case("rename"),
        _ => {
            let text = text.trim().to_lowercase();
            text.starts_with("переим") || text.starts_with("rename")
        }
    }
}

fn name_of(folder: &IShellFolder, child: *const ITEMIDLIST, flags: SHGDNF) -> Option<String> {
    unsafe {
        let mut strret = STRRET::default();
        folder.GetDisplayNameOf(child, flags, &mut strret).ok()?;
        let mut text = PWSTR::null();
        StrRetToStrW(&mut strret, Some(child), &mut text).ok()?;
        let name = text.to_string().ok();
        CoTaskMemFree(Some(text.0 as _));
        name
    }
}

fn with_parent<R>(item: &IShellItem, f: impl FnOnce(&IShellFolder, *const ITEMIDLIST) -> R) -> Option<R> {
    unsafe {
        let pidl = SHGetIDListFromObject(item).ok()?;
        let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
        let result = SHBindToParent::<IShellFolder>(pidl, Some(&mut child)).ok().map(|folder| f(&folder, child));
        CoTaskMemFree(Some(pidl as _));
        result
    }
}

pub fn editing_name(item: &IShellItem) -> Option<String> {
    with_parent(item, |folder, child| name_of(folder, child, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FOREDITING.0))).flatten()
}

pub fn full_name(parsing: &str, editing: &str, new: &str) -> String {
    let hidden = match parsing.strip_prefix(editing) {
        Some(rest) if !rest.is_empty() && rest.starts_with('.') => rest,
        _ => "",
    };
    if hidden.is_empty() || new.to_lowercase().ends_with(&hidden.to_lowercase()) {
        new.to_string()
    } else {
        format!("{new}{hidden}")
    }
}

fn rename_with_operation(item: &IShellItem, full: &str) -> Result<(String, IShellItem)> {
    unsafe {
        let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        operation.SetOperationFlags(FOF_ALLOWUNDO | FOFX_EARLYFAILURE)?;
        operation.RenameItem(item, &HSTRING::from(full), None)?;
        operation.PerformOperations()?;
        if operation.GetAnyOperationsAborted()?.as_bool() {
            return Err(windows::core::Error::empty());
        }
        let parent = item.GetParent()?;
        let folder = parent.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = std::path::Path::new(&folder.to_string().unwrap_or_default()).join(full);
        CoTaskMemFree(Some(folder.0 as _));
        let fresh = item_at(&path).ok_or_else(windows::core::Error::empty)?;
        Ok((full.to_string(), fresh))
    }
}

pub fn target_name(item: &IShellItem, name: &str) -> Option<String> {
    let (parsing, editing) = with_parent(item, |folder, child| {
        (
            name_of(folder, child, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0)),
            name_of(folder, child, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FOREDITING.0)),
        )
    })?;
    Some(full_name(&parsing?, &editing?, name))
}

pub fn rename(item: &IShellItem, name: &str) -> Result<(String, IShellItem)> {
    let names = with_parent(item, |folder, child| {
        (
            name_of(folder, child, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0)),
            name_of(folder, child, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FOREDITING.0)),
        )
    });
    let (parsing, editing) = match names {
        Some((Some(parsing), Some(editing))) => (parsing, editing),
        _ => return Err(windows::core::Error::empty()),
    };
    if editing == name {
        return Ok((parsing, item.clone()));
    }
    match rename_in_folder(item, name) {
        Ok(renamed) => Ok(renamed),
        Err(first) => rename_with_operation(item, &full_name(&parsing, &editing, name)).map_err(|_| first),
    }
}

fn rename_in_folder(item: &IShellItem, name: &str) -> Result<(String, IShellItem)> {
    with_parent(item, |folder, child| unsafe {
        let mut renamed: *mut ITEMIDLIST = std::ptr::null_mut();
        folder.SetNameOf(None, child, &HSTRING::from(name), SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FOREDITING.0), Some(&mut renamed))?;
        let parsed = name_of(folder, renamed, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0));
        let fresh: Result<IShellItem> = SHCreateItemWithParent(None, folder, renamed);
        CoTaskMemFree(Some(renamed as _));
        Ok((parsed.ok_or_else(windows::core::Error::empty)?, fresh?))
    })
    .unwrap_or_else(|| Err(windows::core::Error::empty()))
}

pub fn context_menu(owner: HWND, item: &IShellItem, extra: Option<&str>, at: POINT) -> Option<MenuChoice> {
    unsafe {
        let menu: IContextMenu = item.BindToHandler(None, &BHID_SFUIObject).ok()?;
        let popup = CreatePopupMenu().ok()?;
        let mut first = 0;
        if let Some(extra) = extra {
            _ = AppendMenuW(popup, MF_STRING, EXTRA_COMMAND as usize, &HSTRING::from(extra));
            _ = AppendMenuW(popup, MF_SEPARATOR, 0, None);
            first = 2;
        }
        if menu.QueryContextMenu(popup, first, FIRST_COMMAND, LAST_COMMAND, CMF_NORMAL | CMF_CANRENAME).is_err() {
            _ = DestroyMenu(popup);
            return None;
        }

        ACTIVE_MENU.with(|m| *m.borrow_mut() = Some(menu.clone()));
        _ = SetForegroundWindow(owner);
        let command = TrackPopupMenuEx(popup, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, at.x, at.y, owner, None).0 as u32;
        ACTIVE_MENU.with(|m| *m.borrow_mut() = None);
        let picked = (FIRST_COMMAND..=LAST_COMMAND).contains(&command).then(|| verb(&menu, (command - FIRST_COMMAND) as usize));
        let text = item_text(popup, command);

        let choice = if command == EXTRA_COMMAND {
            Some(MenuChoice::Extra)
        } else if (FIRST_COMMAND..=LAST_COMMAND).contains(&command) && is_rename(picked.flatten().as_deref(), &text) {
            Some(MenuChoice::Rename)
        } else if (FIRST_COMMAND..=LAST_COMMAND).contains(&command) {
            let verb = (command - FIRST_COMMAND) as usize;
            let info = CMINVOKECOMMANDINFOEX {
                cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE,
                hwnd: owner,
                lpVerb: PCSTR(verb as *const u8),
                lpVerbW: PCWSTR(verb as *const u16),
                nShow: SW_SHOWNORMAL.0,
                ptInvoke: at,
                ..Default::default()
            };
            _ = menu.InvokeCommand(&info as *const _ as *const _);
            Some(MenuChoice::Done)
        } else {
            None
        };
        _ = DestroyMenu(popup);
        choice
    }
}

pub fn menu_open() -> bool {
    ACTIVE_MENU.with(|m| m.try_borrow().map_or(true, |m| m.is_some()))
}

pub fn forward_menu_message(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let menu = ACTIVE_MENU.with(|m| m.borrow().clone())?;
    unsafe {
        if let Ok(menu3) = menu.cast::<IContextMenu3>() {
            let mut result = LRESULT(0);
            return menu3.HandleMenuMsg2(msg, wparam, lparam, Some(&mut result)).ok().map(|_| result);
        }
        let menu2 = menu.cast::<IContextMenu2>().ok()?;
        menu2.HandleMenuMsg(msg, wparam, lparam).ok().map(|_| LRESULT(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_extensions_are_kept() {
        assert_eq!(full_name("report.pdf", "report", "summary"), "summary.pdf");
        assert_eq!(full_name("report.pdf", "report", "summary.pdf"), "summary.pdf");
        assert_eq!(full_name("report.pdf", "report.pdf", "summary.txt"), "summary.txt");
        assert_eq!(full_name("Mucic", "Mucic", "Music"), "Music");
        assert_eq!(full_name("v1.2", "v1.2", "v1.3"), "v1.3");
    }

    #[test]
    fn only_pictures_and_video_get_previews() {
        assert!(shows_preview("photo.JPG") && shows_preview("clip.mp4"));
        assert!(!shows_preview("report.pdf") && !shows_preview("table.xlsx") && !shows_preview("Tool.lnk"));
        assert!(!shows_preview("Folder") && !shows_preview("::{GUID}"));
    }

    #[test]
    fn icons_without_alpha_become_opaque() {
        let mut pixels = vec![200, 100, 50, 0, 10, 20, 30, 0];
        normalize(&mut pixels);
        assert_eq!(pixels, [200, 100, 50, 255, 10, 20, 30, 255]);
    }

    #[test]
    fn straight_alpha_is_premultiplied_once() {
        let mut straight = vec![255, 255, 255, 128, 0, 0, 0, 0];
        normalize(&mut straight);
        assert_eq!(straight, [128, 128, 128, 128, 0, 0, 0, 0]);
        let mut premultiplied = vec![100, 100, 100, 128, 0, 0, 0, 0];
        normalize(&mut premultiplied);
        assert_eq!(premultiplied, [100, 100, 100, 128, 0, 0, 0, 0]);
    }

    #[test]
    fn rename_is_found_by_verb_or_by_text() {
        assert!(is_rename(Some("rename"), "что угодно"));
        assert!(!is_rename(Some("delete"), "Переименовать"));
        assert!(is_rename(None, "Переименовать"));
        assert!(is_rename(None, "Переим."));
        assert!(is_rename(Some(""), "Rename\tF2"));
        assert!(!is_rename(None, "Удалить"));
    }
}
