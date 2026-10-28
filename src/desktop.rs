use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, ScreenToClient,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, IServiceProvider};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::Common::{ITEMIDLIST, STRRET};
use windows::Win32::UI::Shell::{
    CSIDL_DESKTOP, FWF_AUTOARRANGE, IFolderView2, IShellBrowser, IShellFolder, IShellItem, IShellWindows,
    SHCreateItemWithParent, SHGDN_FORPARSING,
    SHGDN_FOREDITING, SHGDN_INFOLDER, SHGDNF, SID_STopLevelBrowser, SVGIO_ALLVIEW, SVSI_POSITIONITEM, SWC_DESKTOP,
    SWFO_NEEDDISPATCH, ShellWindows, StrRetToStrW,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetParent};
use windows::core::{Interface, PWSTR, Result, w};

pub struct Desktop {
    view: IFolderView2,
    folder: IShellFolder,
    pub host: HWND,
    pub def_view: HWND,
    list_view: HWND,
}

pub struct DesktopItem {
    pub name: String,
    pub position: POINT,
    pidl: *mut ITEMIDLIST,
}

impl Drop for DesktopItem {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.pidl as _)) };
    }
}

impl Desktop {
    pub fn connect() -> Result<Desktop> {
        unsafe {
            let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
            let location = VARIANT::from(CSIDL_DESKTOP as i32);
            let empty = VARIANT::default();
            let mut hwnd = 0i32;
            let dispatch = windows.FindWindowSW(&location, &empty, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)?;
            let provider: IServiceProvider = dispatch.cast()?;
            let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
            let shell_view = browser.QueryActiveShellView()?;
            let view: IFolderView2 = shell_view.cast()?;
            let folder: IShellFolder = view.GetFolder()?;
            let def_view = shell_view.GetWindow()?;
            let list_view = FindWindowExW(Some(def_view), None, w!("SysListView32"), None)?;
            let host = GetParent(def_view)?;
            Ok(Desktop { view, folder, host, def_view, list_view })
        }
    }

    pub fn list_view(&self) -> HWND {
        self.list_view
    }

    pub fn shell_item(&self, item: &DesktopItem) -> Result<IShellItem> {
        unsafe { SHCreateItemWithParent(None, &self.folder, item.pidl) }
    }

    pub fn disable_auto_arrange(&self) -> Result<()> {
        let mask = FWF_AUTOARRANGE.0 as u32;
        unsafe {
            if self.view.GetCurrentFolderFlags()? & mask != 0 {
                self.view.SetCurrentFolderFlags(mask, 0)?;
            }
        }
        Ok(())
    }

    pub fn spacing(&self) -> Result<POINT> {
        let mut spacing = POINT::default();
        unsafe { self.view.GetSpacing(&mut spacing)? };
        Ok(spacing)
    }

    pub fn items(&self) -> Result<Vec<DesktopItem>> {
        let mut items = Vec::new();
        unsafe {
            let count = self.view.ItemCount(SVGIO_ALLVIEW)?;
            for index in 0..count {
                let Ok(pidl) = self.view.Item(index) else { continue };
                let mut item = DesktopItem { name: String::new(), position: POINT::default(), pidl };
                let (Ok(name), Ok(mut position)) = (self.display_name(pidl), self.view.GetItemPosition(pidl)) else {
                    continue;
                };
                _ = ClientToScreen(self.list_view, &mut position);
                item.name = name;
                item.position = position;
                items.push(item);
            }
        }
        Ok(items)
    }

    pub fn editing_name(&self, item: &DesktopItem) -> Result<String> {
        self.name_with(item.pidl, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FOREDITING.0))
    }

    pub fn rename(&self, item: &DesktopItem, name: &str) -> Result<String> {
        unsafe {
            let mut renamed: *mut ITEMIDLIST = std::ptr::null_mut();
            self.folder.SetNameOf(
                None, item.pidl, &windows::core::HSTRING::from(name), SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FOREDITING.0),
                Some(&mut renamed),
            )?;
            let result = self.display_name(renamed);
            CoTaskMemFree(Some(renamed as _));
            result
        }
    }

    fn display_name(&self, pidl: *const ITEMIDLIST) -> Result<String> {
        self.name_with(pidl, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0))
    }

    fn name_with(&self, pidl: *const ITEMIDLIST, flags: SHGDNF) -> Result<String> {
        unsafe {
            let mut strret = STRRET::default();
            self.folder.GetDisplayNameOf(pidl, flags, &mut strret)?;
            let mut text = PWSTR::null();
            StrRetToStrW(&mut strret, Some(pidl), &mut text)?;
            let name = text.to_string().unwrap_or_default();
            CoTaskMemFree(Some(text.0 as _));
            Ok(name)
        }
    }

    pub fn position(&self, moves: &[(&DesktopItem, POINT)]) -> Result<()> {
        if moves.is_empty() {
            return Ok(());
        }
        let pidls: Vec<*const ITEMIDLIST> = moves.iter().map(|(item, _)| item.pidl as *const _).collect();
        let points: Vec<POINT> = moves
            .iter()
            .map(|&(_, screen)| {
                let mut point = screen;
                unsafe { _ = ScreenToClient(self.list_view, &mut point) };
                point
            })
            .collect();
        unsafe {
            self.view.SelectAndPositionItems(
                pidls.len() as u32,
                pidls.as_ptr(),
                Some(points.as_ptr()),
                SVSI_POSITIONITEM.0 as u32,
            )
        }
    }
}

pub fn work_area(point: POINT) -> RECT {
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        _ = GetMonitorInfoW(monitor, &mut info);
        info.rcWork
    }
}
