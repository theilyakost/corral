use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{DeleteObject, HPALETTE};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, IWICBitmap, IWICImagingFactory, WICBitmapUsePremultipliedAlpha,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, CMF_NORMAL, CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFOEX, IContextMenu,
    IContextMenu2, IContextMenu3, IShellItem, IShellItemImageFactory, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW,
    SHGetIDListFromObject, SIGDN_NORMALDISPLAY, SIIGBF_RESIZETOFIT, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL, SetForegroundWindow,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
};
use windows::core::{HSTRING, Interface, PCSTR, PCWSTR};

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

pub fn icon(item: &IShellItem, size: i32) -> Option<IWICBitmap> {
    unsafe {
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let bitmap = factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_RESIZETOFIT).ok()?;
        let wic = WIC.with(|cell| {
            let mut cell = cell.borrow_mut();
            if cell.is_none() {
                *cell = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok();
            }
            cell.clone()
        });
        let result = wic.and_then(|wic| wic.CreateBitmapFromHBITMAP(bitmap, HPALETTE::default(), WICBitmapUsePremultipliedAlpha).ok());
        _ = DeleteObject(bitmap.into());
        result
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
    Done,
}

pub fn context_menu(owner: HWND, item: &IShellItem, extra: &str, at: POINT) -> Option<MenuChoice> {
    unsafe {
        let menu: IContextMenu = item.BindToHandler(None, &BHID_SFUIObject).ok()?;
        let popup = CreatePopupMenu().ok()?;
        _ = AppendMenuW(popup, MF_STRING, EXTRA_COMMAND as usize, &HSTRING::from(extra));
        _ = AppendMenuW(popup, MF_SEPARATOR, 0, None);
        if menu.QueryContextMenu(popup, 2, FIRST_COMMAND, LAST_COMMAND, CMF_NORMAL).is_err() {
            _ = DestroyMenu(popup);
            return None;
        }

        ACTIVE_MENU.with(|m| *m.borrow_mut() = Some(menu.clone()));
        _ = SetForegroundWindow(owner);
        let command = TrackPopupMenuEx(popup, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, at.x, at.y, owner, None).0 as u32;
        ACTIVE_MENU.with(|m| *m.borrow_mut() = None);

        let choice = if command == EXTRA_COMMAND {
            Some(MenuChoice::Extra)
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
