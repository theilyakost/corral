use std::sync::OnceLock;

#[cfg(windows)]
use winapi::shared::windef::{HWND, RECT, POINT};
#[cfg(windows)]
use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM, TRUE, FALSE, MAX_PATH};
#[cfg(windows)]
use winapi::um::winuser::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    PostQuitMessage, TranslateMessage, SetWindowPos, LoadCursorW, PostMessageW,
    BeginPaint, EndPaint, FillRect, GetClientRect, SystemParametersInfoW, InvalidateRect,
    WM_DESTROY, WM_PAINT, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_LBUTTONDBLCLK,
    WM_MOUSEMOVE, WM_MOUSELEAVE, WM_SETCURSOR, WM_WINDOWPOSCHANGING, WM_MOVING, WM_CREATE, WM_DROPFILES,
    WM_MOUSEACTIVATE, WM_APP,
    WS_POPUP, WS_VISIBLE, WS_EX_TOOLWINDOW, WS_EX_LAYERED, WS_EX_ACCEPTFILES,
    CS_HREDRAW, CS_VREDRAW, CS_DBLCLKS,
    WNDCLASSW, MSG, IDC_ARROW, MA_NOACTIVATE,
    ReleaseCapture, SetCapture, SendMessageW, WM_SYSCOMMAND,
    HWND_BOTTOM, SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE, WINDOWPOS,
    SetLayeredWindowAttributes, LWA_ALPHA, SPI_GETWORKAREA,
    DT_CENTER, DT_TOP, DT_WORDBREAK, DT_NOPREFIX, DT_END_ELLIPSIS, DrawTextW,
    DrawIconEx, DestroyIcon, GetCursorPos, TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE, GetWindowRect
};
#[cfg(windows)]
use winapi::um::wingdi::{
    CreateSolidBrush, DeleteObject, SetBkMode, SetTextColor, TRANSPARENT,
    SelectObject, CreateFontW, FW_NORMAL
};
#[cfg(windows)]
use winapi::um::libloaderapi::GetModuleHandleW;
#[cfg(windows)]
use winapi::um::shellapi::{
    DragAcceptFiles, DragQueryFileW, DragFinish, HDROP,
    SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON,
    ShellExecuteW
};
#[cfg(windows)]
use winapi::um::shlobj::{SHGetSpecialFolderPathW, CSIDL_DESKTOP};
#[cfg(windows)]
use winapi::um::fileapi::{FindFirstChangeNotificationW, FindNextChangeNotification};
#[cfg(windows)]
use winapi::um::synchapi::WaitForSingleObject;
#[cfg(windows)]
use winapi::um::winbase::{MoveFileExW, MOVEFILE_COPY_ALLOWED, INFINITE, WAIT_OBJECT_0};
#[cfg(windows)]
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
#[cfg(windows)]
use winapi::um::winnt::{FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_DIR_NAME};

#[cfg(windows)]
use std::ffi::{OsStr, OsString};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::ptr::null_mut;
#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::fs;

const DI_NORMAL: u32 = 0x0003;

#[cfg(windows)]
const WM_APP_REFRESH: UINT = WM_APP + 1;

struct AppState {
    drag_source: Option<PathBuf>,
    hover_index: Option<usize>,
    is_dragging_window: bool,
    files: Vec<PathBuf>,
}

static mut STATE: AppState = AppState {
    drag_source: None,
    hover_index: None,
    is_dragging_window: false,
    files: Vec::new(),
};

static STORE_DIR: OnceLock<PathBuf> = OnceLock::new();

#[cfg(windows)]
fn to_wstring(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn lparam_xy(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam & 0xFFFF) as i16 as i32;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
    (x, y)
}

#[cfg(windows)]
fn store_dir_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("corral").join("Files")
}

#[cfg(windows)]
fn read_files() -> Vec<PathBuf> {
    if let Some(store_dir) = STORE_DIR.get() {
        if let Ok(entries) = fs::read_dir(store_dir) {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .collect();
            files.sort();
            return files;
        }
    }
    Vec::new()
}

#[cfg(windows)]
fn unique_dest(dir: &Path, file_name: &OsStr) -> Option<PathBuf> {
    let first = dir.join(file_name);
    if !first.try_exists().unwrap_or(true) {
        return Some(first);
    }
    let name = Path::new(file_name);
    let stem = name.file_stem().unwrap_or(file_name);
    for n in 2..10_000 {
        let mut candidate = OsString::from(stem);
        candidate.push(format!(" ({n})"));
        if let Some(ext) = name.extension() {
            candidate.push(".");
            candidate.push(ext);
        }
        let path = dir.join(candidate);
        if !path.try_exists().unwrap_or(true) {
            return Some(path);
        }
    }
    None
}

#[cfg(windows)]
fn move_into(src: &Path, dir: &Path) -> bool {
    if src.parent() == Some(dir) {
        return false;
    }
    let Some(file_name) = src.file_name() else { return false };
    let Some(dest) = unique_dest(dir, file_name) else { return false };
    let src_w = to_wstring(src);
    let dest_w = to_wstring(&dest);
    unsafe { MoveFileExW(src_w.as_ptr(), dest_w.as_ptr(), MOVEFILE_COPY_ALLOWED) != 0 }
}

#[cfg(windows)]
fn spawn_watcher(hwnd: HWND, dir: PathBuf) {
    let hwnd = hwnd as usize;
    std::thread::spawn(move || unsafe {
        let dir_w = to_wstring(&dir);
        let handle = FindFirstChangeNotificationW(
            dir_w.as_ptr(), FALSE, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME,
        );
        if handle == INVALID_HANDLE_VALUE { return; }
        while WaitForSingleObject(handle, INFINITE) == WAIT_OBJECT_0 {
            PostMessageW(hwnd as HWND, WM_APP_REFRESH, 0, 0);
            if FindNextChangeNotification(handle) == 0 { break; }
        }
    });
}

#[cfg(windows)]
fn refresh_files(state: &mut AppState, hwnd: HWND) {
    state.files = read_files();
    if state.hover_index.is_some_and(|i| i >= state.files.len()) {
        state.hover_index = None;
    }
    unsafe { InvalidateRect(hwnd, null_mut(), TRUE); }
}

#[cfg(windows)]
fn hit_test(x: i32, y: i32, client_w: i32, files_count: usize) -> Option<usize> {
    let cell_w = 70;
    let cell_h = 80;
    let cols = std::cmp::max(1, client_w / cell_w);

    if x < 0 || y < 0 { return None; }

    let col = x / cell_w;
    let row = y / cell_h;

    if col >= cols { return None; }

    let index = row * cols + col;

    if (index as usize) < files_count {
        Some(index as usize)
    } else {
        None
    }
}

#[cfg(windows)]
fn get_desktop_path() -> PathBuf {
    let mut buf = [0u16; MAX_PATH];
    unsafe {
        SHGetSpecialFolderPathW(null_mut(), buf.as_mut_ptr(), CSIDL_DESKTOP, FALSE);
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(MAX_PATH);
    PathBuf::from(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(windows)]
unsafe extern "system" fn window_proc(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let state = unsafe { &mut *std::ptr::addr_of_mut!(STATE) };

    match msg {
        WM_CREATE => {
            let dir = store_dir_path();
            if fs::create_dir_all(&dir).is_ok() {
                let _ = STORE_DIR.set(dir.clone());
                spawn_watcher(hwnd, dir);
            }
            state.files = read_files();

            unsafe { DragAcceptFiles(hwnd, TRUE); }
            0
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0); }
            0
        }

        WM_APP_REFRESH => {
            refresh_files(state, hwnd);
            0
        }

        WM_DROPFILES => {
            let h_drop = wparam as HDROP;
            unsafe {
                let count = DragQueryFileW(h_drop, 0xFFFFFFFF, null_mut(), 0);
                if let Some(store_dir) = STORE_DIR.get() {
                    for i in 0..count {
                        let len = DragQueryFileW(h_drop, i, null_mut(), 0) + 1;
                        let mut buf = vec![0u16; len as usize];
                        DragQueryFileW(h_drop, i, buf.as_mut_ptr(), len);
                        let path_str = String::from_utf16_lossy(&buf[..len as usize - 1]);
                        move_into(Path::new(&path_str), store_dir);
                    }
                }
                DragFinish(h_drop);
            }
            refresh_files(state, hwnd);
            0
        }

        WM_PAINT => {
            unsafe {
                let mut ps = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                GetClientRect(hwnd, &mut rect);

                let brush = CreateSolidBrush(0x00141414);
                FillRect(hdc, &rect, brush);
                DeleteObject(brush as *mut _);

                SetBkMode(hdc, TRANSPARENT as i32);

                let font_name = to_wstring("Segoe UI");
                let h_font = CreateFontW(16, 0, 0, 0, FW_NORMAL, 0, 0, 0, 0, 0, 0, 2, 0, font_name.as_ptr());
                SelectObject(hdc, h_font as *mut _);

                let icon_size = 32;
                let cell_w = 70;
                let cell_h = 80;
                let cols = std::cmp::max(1, (rect.right - rect.left) / cell_w);

                for (index, full_path) in state.files.iter().enumerate() {
                    let path_vec = to_wstring(full_path);
                    let mut shfi: SHFILEINFOW = std::mem::zeroed();
                    SHGetFileInfoW(path_vec.as_ptr(), 0, &mut shfi, std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | SHGFI_LARGEICON);

                    let col = (index as i32) % cols;
                    let row = (index as i32) / cols;
                    let x = col * cell_w;
                    let y = row * cell_h + 10;

                    if state.hover_index == Some(index) {
                        let hover_rect = RECT { left: x, top: y - 5, right: x + cell_w, bottom: y + cell_h - 5};
                        let hover_brush = CreateSolidBrush(0x00404040);
                        FillRect(hdc, &hover_rect, hover_brush);
                        DeleteObject(hover_brush as *mut _);
                    }

                    if !shfi.hIcon.is_null() {
                        DrawIconEx(hdc, x + (cell_w - icon_size)/2, y, shfi.hIcon, icon_size, icon_size, 0, null_mut(), DI_NORMAL);
                        DestroyIcon(shfi.hIcon);
                    }

                    SetTextColor(hdc, 0x00FFFFFF);
                    let name = full_path.file_name().unwrap_or(full_path.as_os_str());
                    let name_w = to_wstring(name);
                    let mut text_rect = RECT { left: x + 2, top: y + icon_size + 2, right: x + cell_w - 2, bottom: y + cell_h };
                    DrawTextW(hdc, name_w.as_ptr(), -1, &mut text_rect, DT_CENTER | DT_TOP | DT_WORDBREAK | DT_NOPREFIX | DT_END_ELLIPSIS);
                }

                DeleteObject(h_font as *mut _);
                EndPaint(hwnd, &mut ps);
            }
            0
        }

        WM_LBUTTONDOWN => {
            unsafe {
                let (x, y) = lparam_xy(lparam);
                let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                GetClientRect(hwnd, &mut rect);

                if let Some(index) = hit_test(x, y, rect.right, state.files.len()) {
                    state.drag_source = Some(state.files[index].clone());
                    state.is_dragging_window = false;
                    SetCapture(hwnd);
                } else {
                    state.drag_source = None;
                    state.is_dragging_window = true;
                    ReleaseCapture();
                    SendMessageW(hwnd, WM_SYSCOMMAND, 0xF012, 0);
                }
            }
            0
        }

        WM_LBUTTONUP => {
            if let Some(source_path) = state.drag_source.take() {
                unsafe {
                    ReleaseCapture();
                    let mut pt = POINT { x: 0, y: 0 };
                    GetCursorPos(&mut pt);
                    let mut win_rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                    GetWindowRect(hwnd, &mut win_rect);

                    if pt.x < win_rect.left || pt.x > win_rect.right || pt.y < win_rect.top || pt.y > win_rect.bottom {
                        move_into(&source_path, &get_desktop_path());
                        refresh_files(state, hwnd);
                    }
                }
            }
            0
        }

        WM_LBUTTONDBLCLK => {
            unsafe {
                let (x, y) = lparam_xy(lparam);
                let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                GetClientRect(hwnd, &mut rect);

                if let Some(index) = hit_test(x, y, rect.right, state.files.len()) {
                    let path_w = to_wstring(&state.files[index]);
                    let op_w = to_wstring("open");
                    ShellExecuteW(null_mut(), op_w.as_ptr(), path_w.as_ptr(), null_mut(), null_mut(), 1);
                }
            }
            0
        }

        WM_MOUSEMOVE => {
            unsafe {
                let (x, y) = lparam_xy(lparam);
                let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                GetClientRect(hwnd, &mut rect);

                let new_hover = hit_test(x, y, rect.right, state.files.len());

                if new_hover != state.hover_index {
                    state.hover_index = new_hover;
                    InvalidateRect(hwnd, null_mut(), TRUE);
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut tme);
                }
            }
            0
        }

        WM_MOUSELEAVE => {
            state.hover_index = None;
            unsafe { InvalidateRect(hwnd, null_mut(), TRUE); }
            0
        }

        WM_MOVING => {
            unsafe {
                let rect = &mut *(lparam as *mut RECT);
                let mut work_area = RECT { left: 0, top: 0, right: 0, bottom: 0 };
                SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work_area as *mut _ as *mut _, 0);
                let w = rect.right - rect.left;
                let h = rect.bottom - rect.top;
                if rect.left < work_area.left { rect.left = work_area.left; rect.right = rect.left + w; }
                if rect.top < work_area.top { rect.top = work_area.top; rect.bottom = rect.top + h; }
                if rect.right > work_area.right { rect.right = work_area.right; rect.left = rect.right - w; }
                if rect.bottom > work_area.bottom { rect.bottom = work_area.bottom; rect.top = rect.bottom - h; }
            }
            1
        }

        WM_WINDOWPOSCHANGING => {
            unsafe {
                let position = &mut *(lparam as *mut WINDOWPOS);
                position.hwndInsertAfter = HWND_BOTTOM;
                position.flags |= SWP_NOACTIVATE;
            }
            0
        }

        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,

        WM_SETCURSOR => {
            unsafe {
                winapi::um::winuser::SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
            }
            1
        }

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(windows)]
fn main() {
    unsafe {
        let h_instance = GetModuleHandleW(null_mut());
        let class_name = to_wstring("Corral");

        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(window_proc),
            hInstance: h_instance,
            lpszClassName: class_name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wnd_class);

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_ACCEPTFILES,
            class_name.as_ptr(),
            null_mut(),
            WS_POPUP | WS_VISIBLE,
            100, 100, 300, 400,
            null_mut(), null_mut(), h_instance, null_mut(),
        );

        SetLayeredWindowAttributes(hwnd, 0, 180, LWA_ALPHA);
        SetWindowPos(hwnd, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(not(windows))]
fn main() {}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("corral_test_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lparam_negative_coords() {
        let lparam = (((-20i16 as u16 as isize) << 16) | (-100i16 as u16 as isize)) as LPARAM;
        assert_eq!(lparam_xy(lparam), (-100, -20));
    }

    #[test]
    fn move_does_not_overwrite() {
        let src = temp_dir("src");
        let dst = temp_dir("dst");
        fs::write(src.join("a.txt"), "new").unwrap();
        fs::write(dst.join("a.txt"), "old").unwrap();
        fs::write(dst.join("a (2).txt"), "old2").unwrap();

        assert!(move_into(&src.join("a.txt"), &dst));
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "old");
        assert_eq!(fs::read_to_string(dst.join("a (2).txt")).unwrap(), "old2");
        assert_eq!(fs::read_to_string(dst.join("a (3).txt")).unwrap(), "new");
        assert!(!src.join("a.txt").exists());

        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn unique_dest_without_extension() {
        let dir = temp_dir("noext");
        fs::create_dir(dir.join("folder")).unwrap();
        assert_eq!(unique_dest(&dir, OsStr::new("folder")), Some(dir.join("folder (2)")));
        let _ = fs::remove_dir_all(&dir);
    }
}
