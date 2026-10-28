use std::collections::HashSet;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FindFirstChangeNotificationW, FindNextChangeNotification,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::System::Ole::IDropTarget;
use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
    SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::{HSTRING, w};

static WATCHED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

pub struct Entry {
    pub name: String,
    pub folder: bool,
}

pub fn listing(path: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(path) else { return Vec::new() };
    let hidden = FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0;
    let mut entries: Vec<Entry> = read
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            (meta.file_attributes() & hidden == 0)
                .then(|| Entry { name: entry.file_name().to_string_lossy().into_owned(), folder: meta.is_dir() })
        })
        .collect();
    sort(&mut entries);
    entries
}

pub fn sort(entries: &mut [Entry]) {
    entries.sort_by_key(|e| (!e.folder, e.name.to_lowercase()));
}

pub fn items(path: &str) -> Vec<(String, IShellItem)> {
    let root = Path::new(path);
    listing(root)
        .into_iter()
        .filter_map(|entry| {
            let full = HSTRING::from(root.join(&entry.name).as_os_str());
            let item: IShellItem = unsafe { SHCreateItemFromParsingName(&full, None).ok()? };
            Some((entry.name, item))
        })
        .collect()
}

pub fn drop_target(path: &str) -> Option<IDropTarget> {
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        item.BindToHandler(None, &BHID_SFUIObject).ok()
    }
}

pub fn pick_folder(owner: Option<HWND>) -> Option<String> {
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        _ = dialog.SetTitle(w!("Папка для портала"));
        dialog.Show(owner).ok()?;
        let item = dialog.GetResult().ok()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as _));
        text
    }
}

pub fn watch(path: &str, target: HWND, message: u32) {
    {
        let Ok(mut watched) = WATCHED.lock() else { return };
        let watched = watched.get_or_insert_with(HashSet::new);
        if !watched.insert(path.to_lowercase()) {
            return;
        }
    }
    let path = HSTRING::from(path);
    let target = target.0 as isize;
    std::thread::spawn(move || unsafe {
        let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_ATTRIBUTES;
        let Ok(handle) = FindFirstChangeNotificationW(&path, false, filter) else { return };
        while WaitForSingleObject(handle, INFINITE) == WAIT_OBJECT_0 {
            _ = PostMessageW(Some(HWND(target as _)), message, WPARAM(0), LPARAM(0));
            if FindNextChangeNotification(handle).is_err() {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_come_first_then_names() {
        let mut entries = vec![
            Entry { name: "b.txt".into(), folder: false },
            Entry { name: "Zeta".into(), folder: true },
            Entry { name: "A.txt".into(), folder: false },
            Entry { name: "alpha".into(), folder: true },
        ];
        sort(&mut entries);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zeta", "A.txt", "b.txt"]);
    }

    #[test]
    fn listing_reads_a_real_folder() {
        let dir = std::env::temp_dir().join(format!("corral-portal-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("file.txt"), "x").unwrap();
        let names: Vec<String> = listing(&dir).into_iter().map(|e| e.name).collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["sub", "file.txt"]);
        assert!(listing(Path::new(r"Z:\definitely\missing")).is_empty());
    }
}
