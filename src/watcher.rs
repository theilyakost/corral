use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::{
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FindFirstChangeNotificationW,
    FindNextChangeNotification,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};
use windows::Win32::Foundation::WAIT_OBJECT_0;
use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_PublicDesktop, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{GUID, HSTRING};

pub fn watch_desktop(target: HWND, message: u32) {
    for folder in [FOLDERID_Desktop, FOLDERID_PublicDesktop] {
        let Some(path) = known_folder(&folder) else { continue };
        let target = target.0 as isize;
        std::thread::spawn(move || unsafe {
            let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME;
            let Ok(handle) = FindFirstChangeNotificationW(&path, false, filter) else { return };
            while WaitForSingleObject(handle, INFINITE) == WAIT_OBJECT_0 {
                _ = PostMessageW(Some(HWND(target as _)), message, WPARAM(0), LPARAM(0));
                if FindNextChangeNotification(handle).is_err() {
                    break;
                }
            }
        });
    }
}

fn known_folder(id: &GUID) -> Option<HSTRING> {
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let result = path.to_hstring();
        CoTaskMemFree(Some(path.0 as _));
        Some(result)
    }
}
