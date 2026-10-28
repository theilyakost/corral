use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{CloseHandle, FreeLibrary, HANDLE, HMODULE, HWND, LPARAM, LRESULT, POINT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree, IServiceProvider};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::{
    FreeLibraryAndExitThread, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GetModuleHandleExW,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateThread, GetCurrentThreadId, INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, SetEvent,
    THREAD_CREATION_FLAGS, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Controls::{
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDRF_NOTIFYITEMDRAW, CDRF_SKIPDEFAULT, LVHITTESTINFO, LVIF_STATE,
    LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_GETITEMCOUNT, LVM_HITTEST, LVM_SETITEMSTATE, LVN_BEGINLABELEDITW,
    LVN_ITEMCHANGED, LVN_ITEMCHANGING, NM_CUSTOMDRAW, NMCUSTOMDRAW, NMHDR, NMLISTVIEW, NMLVDISPINFOW,
};
use windows::Win32::UI::Shell::Common::STRRET;
use windows::Win32::UI::Shell::{
    CSIDL_DESKTOP, DefSubclassProc, IFolderView2, IShellBrowser, IShellFolder, IShellWindows, RemoveWindowSubclass,
    SHGDN_FORPARSING, SHGDN_INFOLDER, SHGDNF, SID_STopLevelBrowser, SWC_DESKTOP, SWFO_NEEDDISPATCH,
    SetWindowSubclass, ShellWindows, StrRetToStrW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, FindWindowW,
    GetWindowThreadProcessId, HWND_MESSAGE, PostMessageW, RegisterClassW, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
    SendMessageW, UnregisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CONTEXTMENU, WM_COPYDATA, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_NOTIFY, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR, Result, w};

use crate::protocol::{
    self, CMD_CONTEXT_MENU, CMD_DETACH, CMD_DRAG_BEGIN, CMD_DRAG_END, CMD_SET_HIDDEN, CMD_SET_OWNER, EVT_MENU_DONE,
    EVT_RENAME,
};

const SUBCLASS_ID: usize = 0xC0AA;
const WM_OWNER_GONE: u32 = WM_APP + 1;
const WM_SHOW_MENU: u32 = WM_APP + 2;

pub(crate) struct ThreadBound<T>(pub(crate) T);

unsafe impl<T> Sync for ThreadBound<T> {}

struct OwnerWatch {
    process: HANDLE,
    stop: HANDLE,
    window: HWND,
}

struct State {
    module: HMODULE,
    def_view: HWND,
    list_view: HWND,
    window: HWND,
    view: IFolderView2,
    folder: IShellFolder,
    hidden: HashSet<String>,
    watch: Option<(HANDLE, *mut OwnerWatch)>,
    allowed: Option<usize>,
    menu: Option<(String, POINT)>,
}

struct Unload {
    module: HMODULE,
    watch: Option<(HANDLE, *mut OwnerWatch)>,
}

static STARTED: AtomicBool = AtomicBool::new(false);
static STATE: ThreadBound<RefCell<Option<State>>> = ThreadBound(RefCell::new(None));

#[unsafe(no_mangle)]
pub extern "system" fn corral_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if !STARTED.swap(true, Ordering::SeqCst) {
        _ = catch_unwind(|| unsafe { attach() });
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe fn attach() -> Result<()> {
    unsafe {
        let progman = FindWindowW(w!("Progman"), None)?;
        let def_view = FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), None)?;
        let list_view = FindWindowExW(Some(def_view), None, w!("SysListView32"), None)?;
        if GetWindowThreadProcessId(list_view, None) != GetCurrentThreadId() {
            return Ok(());
        }

        let mut module = HMODULE::default();
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            PCWSTR(corral_hook_proc as *const u16),
            &mut module,
        )?;
        let setup = || -> Result<State> {
            let (view, folder) = shell_view()?;
            let class_name = HSTRING::from(protocol::WINDOW_CLASS);
            let class = WNDCLASSW {
                lpfnWndProc: Some(agent_proc),
                hInstance: module.into(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            RegisterClassW(&class);
            let window = CreateWindowExW(
                WINDOW_EX_STYLE::default(), &class_name, None, WINDOW_STYLE::default(),
                0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(module.into()), None,
            )?;
            Ok(State {
                module, def_view, list_view, window, view, folder,
                hidden: HashSet::new(), watch: None, allowed: None, menu: None,
            })
        };
        let state = match setup() {
            Ok(state) => state,
            Err(e) => {
                _ = FreeLibrary(module);
                return Err(e);
            }
        };

        *STATE.0.borrow_mut() = Some(state);
        _ = SetWindowSubclass(def_view, Some(def_view_proc), SUBCLASS_ID, 0);
        _ = SetWindowSubclass(list_view, Some(list_view_proc), SUBCLASS_ID, 0);
        Ok(())
    }
}

unsafe fn shell_view() -> Result<(IFolderView2, IShellFolder)> {
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
        let location = VARIANT::from(CSIDL_DESKTOP as i32);
        let empty = VARIANT::default();
        let mut hwnd = 0i32;
        let dispatch = windows.FindWindowSW(&location, &empty, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)?;
        let provider: IServiceProvider = dispatch.cast()?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        let view: IFolderView2 = browser.QueryActiveShellView()?.cast()?;
        let folder: IShellFolder = view.GetFolder()?;
        Ok((view, folder))
    }
}

fn name_of(state: &State, index: usize) -> Option<String> {
    unsafe {
        let pidl = state.view.Item(index as i32).ok()?;
        let mut strret = STRRET::default();
        let flags = SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0);
        let mut name = None;
        if state.folder.GetDisplayNameOf(pidl, flags, &mut strret).is_ok() {
            let mut text = PWSTR::null();
            if StrRetToStrW(&mut strret, Some(pidl), &mut text).is_ok() {
                name = text.to_string().ok();
                CoTaskMemFree(Some(text.0 as _));
            }
        }
        CoTaskMemFree(Some(pidl as _));
        name
    }
}

pub(crate) fn is_hidden(index: usize) -> bool {
    let Ok(state) = STATE.0.try_borrow() else { return false };
    let Some(state) = state.as_ref() else { return false };
    if state.hidden.is_empty() {
        return false;
    }
    name_of(state, index).is_some_and(|name| state.hidden.contains(&name.to_lowercase()))
}

fn is_allowed(index: usize) -> bool {
    STATE.0.try_borrow().ok().and_then(|s| s.as_ref().map(|s| s.allowed == Some(index))).unwrap_or(false)
}

fn hidden_name(index: usize) -> Option<String> {
    let state = STATE.0.try_borrow().ok()?;
    let state = state.as_ref()?;
    let name = name_of(state, index)?;
    state.hidden.contains(&name.to_lowercase()).then_some(name)
}

fn notify_owner(event: usize, words: &[u16]) {
    unsafe {
        let Ok(main) = FindWindowW(&HSTRING::from(protocol::MAIN_CLASS), None) else { return };
        let payload = COPYDATASTRUCT {
            dwData: event,
            cbData: (words.len() * 2) as u32,
            lpData: if words.is_empty() { std::ptr::null_mut() } else { words.as_ptr() as *mut _ },
        };
        SendMessageTimeoutW(
            main, WM_COPYDATA, WPARAM(0), LPARAM(&payload as *const _ as isize), SMTO_ABORTIFHUNG, 1000, None,
        );
    }
}

fn list_view() -> Option<HWND> {
    STATE.0.try_borrow().ok()?.as_ref().map(|s| s.list_view)
}

unsafe extern "system" fn def_view_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize,
) -> LRESULT {
    if msg == WM_NOTIFY {
        let handled = catch_unwind(AssertUnwindSafe(|| unsafe { notify(hwnd, msg, wparam, lparam) }));
        if let Ok(Some(result)) = handled {
            return result;
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

unsafe fn notify(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    unsafe {
        let header = &*(lparam.0 as *const NMHDR);
        let list_view = list_view()?;
        if header.hwndFrom != list_view {
            return None;
        }
        match header.code {
            NM_CUSTOMDRAW => {
                let draw = &*(lparam.0 as *const NMCUSTOMDRAW);
                if draw.dwDrawStage == CDDS_PREPAINT {
                    let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                    return Some(LRESULT(result.0 | CDRF_NOTIFYITEMDRAW as isize));
                }
                if draw.dwDrawStage == CDDS_ITEMPREPAINT && is_hidden(draw.dwItemSpec) {
                    return Some(LRESULT(CDRF_SKIPDEFAULT as isize));
                }
                None
            }
            LVN_ITEMCHANGING => {
                let change = &*(lparam.0 as *const NMLISTVIEW);
                let selecting = (change.uChanged.0 & LVIF_STATE.0) != 0
                    && (change.uNewState & LVIS_SELECTED.0) != 0
                    && (change.uOldState & LVIS_SELECTED.0) == 0;
                if selecting && change.iItem >= 0 && !is_allowed(change.iItem as usize) && is_hidden(change.iItem as usize) {
                    return Some(LRESULT(1));
                }
                None
            }
            LVN_BEGINLABELEDITW => {
                let info = &*(lparam.0 as *const NMLVDISPINFOW);
                let name = (info.item.iItem >= 0).then(|| hidden_name(info.item.iItem as usize)).flatten()?;
                notify_owner(EVT_RENAME, &protocol::encode_names(&[name]));
                Some(LRESULT(1))
            }
            LVN_ITEMCHANGED => {
                let change = &*(lparam.0 as *const NMLISTVIEW);
                if change.iItem < 0 && (change.uNewState & LVIS_SELECTED.0) != 0 {
                    let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                    deselect_hidden(list_view);
                    return Some(result);
                }
                None
            }
            _ => None,
        }
    }
}

unsafe fn deselect_hidden(list_view: HWND) {
    unsafe {
        let count = SendMessageW(list_view, LVM_GETITEMCOUNT, None, None).0;
        for index in 0..count.max(0) as usize {
            if is_hidden(index) {
                let item = LVITEMW { stateMask: LVIS_SELECTED, ..Default::default() };
                SendMessageW(list_view, LVM_SETITEMSTATE, Some(WPARAM(index)), Some(LPARAM(&item as *const _ as isize)));
            }
        }
    }
}

unsafe extern "system" fn list_view_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize,
) -> LRESULT {
    let is_click = matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN);
    if is_click {
        let blocked = catch_unwind(AssertUnwindSafe(|| unsafe { hits_hidden(hwnd, lparam) }));
        if matches!(blocked, Ok(true)) {
            return LRESULT(0);
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

unsafe fn hits_hidden(hwnd: HWND, lparam: LPARAM) -> bool {
    let point = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
    let mut info = LVHITTESTINFO { pt: point, ..Default::default() };
    let index = unsafe { SendMessageW(hwnd, LVM_HITTEST, None, Some(LPARAM(&mut info as *mut _ as isize))) };
    index.0 >= 0 && is_hidden(index.0 as usize)
}

unsafe extern "system" fn agent_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COPYDATA => {
            let handled = catch_unwind(AssertUnwindSafe(|| unsafe { command(&*(lparam.0 as *const COPYDATASTRUCT)) }));
            LRESULT(handled.is_ok() as isize)
        }
        WM_OWNER_GONE => {
            _ = catch_unwind(|| unsafe { detach() });
            LRESULT(0)
        }
        WM_SHOW_MENU => {
            _ = catch_unwind(|| unsafe { show_menu() });
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn command(data: &COPYDATASTRUCT) {
    let words = if data.lpData.is_null() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(data.lpData as *const u16, data.cbData as usize / 2) }
    };
    match data.dwData {
        CMD_SET_HIDDEN => {
            let names = protocol::decode_names(words);
            let list_view = {
                let Ok(mut state) = STATE.0.try_borrow_mut() else { return };
                let Some(state) = state.as_mut() else { return };
                state.hidden = names.into_iter().map(|n| n.to_lowercase()).collect();
                state.list_view
            };
            unsafe {
                deselect_hidden(list_view);
                _ = InvalidateRect(Some(list_view), None, true);
            }
        }
        CMD_SET_OWNER if words.len() >= 2 => {
            let pid = words[0] as u32 | (words[1] as u32) << 16;
            unsafe { watch_owner(pid) };
        }
        CMD_DRAG_BEGIN => {
            let views = STATE.0.try_borrow().ok().and_then(|s| s.as_ref().map(|s| (s.def_view, s.list_view)));
            if let Some((def_view, list_view)) = views {
                unsafe { crate::drag::begin(def_view, list_view) };
            }
        }
        CMD_DRAG_END => unsafe { crate::drag::end() },
        CMD_CONTEXT_MENU => {
            let Some((x, y)) = protocol::decode_point(words) else { return };
            let Some(name) = protocol::decode_names(&words[4..]).into_iter().next() else { return };
            let Ok(mut state) = STATE.0.try_borrow_mut() else { return };
            let Some(state) = state.as_mut() else { return };
            state.menu = Some((name, POINT { x, y }));
            unsafe { _ = PostMessageW(Some(state.window), WM_SHOW_MENU, WPARAM(0), LPARAM(0)) };
        }
        CMD_DETACH => unsafe { detach() },
        _ => {}
    }
}

unsafe fn set_state(list_view: HWND, index: usize, state: u32) {
    let mask = windows::Win32::UI::Controls::LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0);
    let item = LVITEMW {
        stateMask: mask,
        state: windows::Win32::UI::Controls::LIST_VIEW_ITEM_STATE_FLAGS(state),
        ..Default::default()
    };
    unsafe { SendMessageW(list_view, LVM_SETITEMSTATE, Some(WPARAM(index)), Some(LPARAM(&item as *const _ as isize))) };
}

unsafe fn show_menu() {
    let found = {
        let Ok(mut state) = STATE.0.try_borrow_mut() else { return };
        let Some(state) = state.as_mut() else { return };
        let Some((name, point)) = state.menu.take() else { return };
        let count = unsafe { SendMessageW(state.list_view, LVM_GETITEMCOUNT, None, None).0.max(0) as usize };
        let index = (0..count).find(|&i| name_of(state, i).is_some_and(|n| n.eq_ignore_ascii_case(&name)));
        if let Some(index) = index {
            state.allowed = Some(index);
        }
        index.map(|index| (state.list_view, index, point))
    };
    let Some((list_view, index, _point)) = found else {
        notify_owner(EVT_MENU_DONE, &[]);
        return;
    };
    unsafe {
        set_state(list_view, usize::MAX, 0);
        set_state(list_view, index, LVIS_SELECTED.0 | LVIS_FOCUSED.0);
        SendMessageW(list_view, WM_CONTEXTMENU, Some(WPARAM(list_view.0 as usize)), Some(LPARAM(-1)));
        if let Ok(mut state) = STATE.0.try_borrow_mut() {
            if let Some(state) = state.as_mut() {
                state.allowed = None;
            }
        }
        set_state(list_view, index, 0);
    }
    notify_owner(EVT_MENU_DONE, &[]);
}

unsafe fn watch_owner(pid: u32) {
    let Ok(mut state) = STATE.0.try_borrow_mut() else { return };
    let Some(state) = state.as_mut() else { return };
    if state.watch.is_some() {
        return;
    }
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else { return };
        let Ok(stop) = CreateEventW(None, true, false, None) else {
            _ = CloseHandle(process);
            return;
        };
        let context = Box::into_raw(Box::new(OwnerWatch { process, stop, window: state.window }));
        match CreateThread(None, 0, Some(owner_thread), Some(context as *const c_void), THREAD_CREATION_FLAGS(0), None) {
            Ok(thread) => state.watch = Some((thread, context)),
            Err(_) => {
                let context = Box::from_raw(context);
                _ = CloseHandle(context.process);
                _ = CloseHandle(context.stop);
            }
        }
    }
}

unsafe extern "system" fn owner_thread(context: *mut c_void) -> u32 {
    unsafe {
        let context = &*(context as *const OwnerWatch);
        if WaitForMultipleObjects(&[context.process, context.stop], false, INFINITE) == WAIT_OBJECT_0 {
            _ = PostMessageW(Some(context.window), WM_OWNER_GONE, WPARAM(0), LPARAM(0));
        }
    }
    0
}

unsafe fn detach() {
    unsafe { crate::drag::end() };
    let Some(state) = STATE.0.try_borrow_mut().ok().and_then(|mut s| s.take()) else { return };
    unsafe {
        _ = RemoveWindowSubclass(state.def_view, Some(def_view_proc), SUBCLASS_ID);
        _ = RemoveWindowSubclass(state.list_view, Some(list_view_proc), SUBCLASS_ID);
        _ = InvalidateRect(Some(state.list_view), None, true);
        _ = DestroyWindow(state.window);
        if let Some((_, context)) = state.watch {
            _ = SetEvent((*context).stop);
        }
        let unload_context = Box::into_raw(Box::new(Unload { module: state.module, watch: state.watch }));
        drop(state);
        _ = CreateThread(None, 0, Some(unload), Some(unload_context as *const c_void), THREAD_CREATION_FLAGS(0), None);
    }
}

unsafe extern "system" fn unload(context: *mut c_void) -> u32 {
    unsafe {
        let context = Box::from_raw(context as *mut Unload);
        if let Some((thread, watch)) = context.watch {
            WaitForSingleObject(thread, INFINITE);
            _ = CloseHandle(thread);
            let watch = Box::from_raw(watch);
            _ = CloseHandle(watch.process);
            _ = CloseHandle(watch.stop);
        }
        let module = context.module;
        drop(context);
        _ = UnregisterClassW(&HSTRING::from(protocol::WINDOW_CLASS), Some(module.into()));
        FreeLibraryAndExitThread(module, 0)
    }
}
