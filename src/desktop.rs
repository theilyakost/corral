use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, ScreenToClient,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, IServiceProvider};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::Common::{ITEMIDLIST, STRRET};
use windows::Win32::UI::Shell::{
    CSIDL_DESKTOP, FWF_AUTOARRANGE, FWF_SNAPTOGRID, IFolderView2, IShellBrowser, IShellFolder, IShellWindows,
    SHGDN_FORPARSING, SHGDN_INFOLDER, SHGDNF, SID_STopLevelBrowser, SVGIO_ALLVIEW, SWC_DESKTOP, SWFO_NEEDDISPATCH,
    ShellWindows, StrRetToStrW,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetParent, SendMessageW};
use windows::core::{Interface, PWSTR, Result, w};

const LVM_SETITEMPOSITION: u32 = 0x100F;

pub struct Desktop {
    view: IFolderView2,
    folder: IShellFolder,
    pub host: HWND,
    pub def_view: HWND,
    list_view: HWND,
}

pub struct DesktopItem {
    pub name: String,
    pub index: i32,
    pub position: POINT,
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

    pub fn disable_auto_arrange(&self) -> Result<()> {
        let mask = (FWF_AUTOARRANGE.0 | FWF_SNAPTOGRID.0) as u32;
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
                let name = self.display_name(pidl);
                let position = self.view.GetItemPosition(pidl);
                CoTaskMemFree(Some(pidl as _));
                if let (Ok(name), Ok(mut position)) = (name, position) {
                    _ = ClientToScreen(self.list_view, &mut position);
                    items.push(DesktopItem { name, index, position });
                }
            }
        }
        Ok(items)
    }

    fn display_name(&self, pidl: *const ITEMIDLIST) -> Result<String> {
        unsafe {
            let mut strret = STRRET::default();
            let flags = SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0);
            self.folder.GetDisplayNameOf(pidl, flags, &mut strret)?;
            let mut text = PWSTR::null();
            StrRetToStrW(&mut strret, Some(pidl), &mut text)?;
            let name = text.to_string().unwrap_or_default();
            CoTaskMemFree(Some(text.0 as _));
            Ok(name)
        }
    }

    pub fn move_item(&self, index: i32, screen: POINT) {
        let mut point = screen;
        unsafe {
            _ = ScreenToClient(self.list_view, &mut point);
            let packed = ((point.y & 0xFFFF) << 16) | (point.x & 0xFFFF);
            SendMessageW(
                self.list_view,
                LVM_SETITEMPOSITION,
                Some(WPARAM(index as usize)),
                Some(LPARAM(packed as isize)),
            );
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
