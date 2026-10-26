#![windows_subsystem = "windows"]

mod config;
mod desktop;
mod group;
mod layout;
mod watcher;

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, ShellExecuteW, Shell_NotifyIconW};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    GetCursorPos, GetMessageW, IDI_APPLICATION, KillTimer, LoadIconW, MB_ICONERROR, MF_SEPARATOR, MF_STRING, MSG,
    MessageBoxW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_SHOWNORMAL, SetForegroundWindow,
    SetTimer, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CONTEXTMENU, WM_DESTROY,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{HSTRING, w};

use config::{Config, Group};
use desktop::{Desktop, DesktopItem};
use group::GroupWindows;
use layout::{Grid, Icon};

const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_DESKTOP_CHANGED: u32 = WM_APP + 2;
const TIMER_LAYOUT: usize = 1;
const TIMER_ATTACH: usize = 2;
const CMD_OPEN_CONFIG: usize = 1;
const CMD_RELOAD: usize = 2;
const CMD_EXIT: usize = 3;
const MAX_RETRIES: u32 = 3;

struct App {
    main: HWND,
    instance: HINSTANCE,
    config: Config,
    windows: Vec<GroupWindows>,
    drag: Option<Drag>,
    retries: u32,
}

struct Snapshot {
    items: Vec<DesktopItem>,
    grid: Grid,
    applied: Option<POINT>,
}

#[derive(Clone, Copy)]
struct Drag {
    index: usize,
    cursor: POINT,
    x: i32,
    y: i32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static DESKTOP: RefCell<Option<Desktop>> = const { RefCell::new(None) };
    static SNAPSHOT: RefCell<Option<Snapshot>> = const { RefCell::new(None) };
}

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut app| app.as_mut().map(f)))
}

fn schedule(timer: usize, delay_ms: u32) {
    if let Some(main) = with_app(|app| app.main) {
        unsafe { SetTimer(Some(main), timer, delay_ms, None) };
    }
}

fn show_error(text: &str) {
    unsafe { MessageBoxW(None, &HSTRING::from(text), w!("Corral"), MB_ICONERROR) };
}

fn main() {
    unsafe {
        _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let _guard = CreateMutexW(None, true, w!("Local\\CorralSingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }

        let config = match Config::load_or_create() {
            Ok(config) => config,
            Err(e) => return show_error(&format!("Не удалось прочитать конфиг:\n{e}")),
        };

        let instance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        group::register_classes(instance);
        let class = WNDCLASSW {
            lpfnWndProc: Some(main_proc),
            hInstance: instance,
            lpszClassName: w!("CorralMain"),
            ..Default::default()
        };
        RegisterClassW(&class);
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")), Ordering::Relaxed);

        let Ok(main) = CreateWindowExW(
            WS_EX_TOOLWINDOW, w!("CorralMain"), w!("Corral"), WS_POPUP,
            0, 0, 0, 0, None, None, Some(instance), None,
        ) else {
            return show_error("Не удалось создать окно");
        };

        APP.with(|cell| {
            *cell.borrow_mut() = Some(App { main, instance, config, windows: Vec::new(), drag: None, retries: 0 });
        });
        add_tray_icon(main);
        watcher::watch_desktop(main, WM_APP_DESKTOP_CHANGED);
        attach();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn attach() {
    let Ok(desktop) = Desktop::connect() else {
        return schedule(TIMER_ATTACH, 2000);
    };
    _ = desktop.disable_auto_arrange();
    let (host, def_view) = (desktop.host, desktop.def_view);
    let stored = DESKTOP.with(|cell| cell.try_borrow_mut().map(|mut d| *d = Some(desktop)).is_ok());
    if !stored {
        return schedule(TIMER_ATTACH, 200);
    }
    rebuild_windows(host, def_view);
    layout_icons();
}

fn detach() {
    DESKTOP.with(|cell| {
        if let Ok(mut desktop) = cell.try_borrow_mut() {
            *desktop = None;
        }
    });
}

fn rebuild_windows(host: HWND, def_view: HWND) {
    let Some((old, groups, instance)) =
        with_app(|app| (std::mem::take(&mut app.windows), app.config.groups.clone(), app.instance))
    else {
        return;
    };
    for windows in old {
        windows.destroy();
    }
    let created: Vec<GroupWindows> = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| GroupWindows::create(index, group, host, def_view, instance).ok())
        .collect();
    with_app(|app| app.windows = created);
}

fn layout_icons() {
    let Some(groups) = with_app(|app| app.config.groups.clone()) else {
        return schedule(TIMER_LAYOUT, 200);
    };
    let result = DESKTOP.with(|cell| match cell.try_borrow() {
        Ok(desktop) => desktop.as_ref().map(|d| arrange(d, &groups).map(Some)),
        Err(_) => Some(Ok(None)),
    });
    match result {
        None => {}
        Some(Ok(None)) => schedule(TIMER_LAYOUT, 200),
        Some(Ok(Some(0))) => _ = with_app(|app| app.retries = 0),
        Some(Ok(Some(_))) => {
            let retry = with_app(|app| {
                app.retries += 1;
                app.retries <= MAX_RETRIES
            });
            if retry == Some(true) {
                schedule(TIMER_LAYOUT, 600);
            } else {
                with_app(|app| app.retries = 0);
            }
        }
        Some(Err(_)) => {
            detach();
            schedule(TIMER_ATTACH, 1500);
        }
    }
}

fn arrange(desktop: &Desktop, groups: &[Group]) -> windows::core::Result<usize> {
    let mut items = desktop.items()?;
    let grid = Grid::detect(items.iter().map(|i| i.position), desktop.spacing()?);
    apply(desktop, &mut items, grid, groups)
}

fn apply(desktop: &Desktop, items: &mut [DesktopItem], grid: Grid, groups: &[Group]) -> windows::core::Result<usize> {
    let icons: Vec<Icon> = items.iter().map(|i| Icon { name: &i.name, position: i.position }).collect();
    let moves = layout::plan(&icons, groups, grid, desktop::work_area);
    let batch: Vec<(&DesktopItem, POINT)> = moves.iter().map(|&(i, target)| (&items[i], target)).collect();
    desktop.position(&batch)?;
    for &(i, target) in &moves {
        items[i].position = target;
    }
    Ok(moves.len())
}

fn take_snapshot() -> Option<Snapshot> {
    DESKTOP.with(|cell| {
        let desktop = cell.try_borrow().ok()?;
        let desktop = desktop.as_ref()?;
        let items = desktop.items().ok()?;
        let grid = Grid::detect(items.iter().map(|i| i.position), desktop.spacing().ok()?);
        Some(Snapshot { items, grid, applied: None })
    })
}

pub fn begin_drag(index: usize) {
    let mut cursor = POINT::default();
    unsafe { _ = GetCursorPos(&mut cursor) };
    with_app(|app| {
        if let Some(group) = app.config.groups.get(index) {
            app.drag = Some(Drag { index, cursor, x: group.x, y: group.y });
        }
    });
    let snapshot = take_snapshot();
    SNAPSHOT.with(|cell| {
        if let Ok(mut current) = cell.try_borrow_mut() {
            *current = snapshot;
        }
    });
}

pub fn drag_to_cursor() {
    let mut cursor = POINT::default();
    unsafe { _ = GetCursorPos(&mut cursor) };
    let moved = with_app(|app| {
        let drag = app.drag?;
        let group = app.config.groups.get_mut(drag.index)?;
        let (x, y) = (drag.x + cursor.x - drag.cursor.x, drag.y + cursor.y - drag.cursor.y);
        if (group.x, group.y) == (x, y) {
            return None;
        }
        group.x = x;
        group.y = y;
        let windows = *app.windows.iter().find(|w| w.index == drag.index)?;
        Some((windows, group.clone(), app.config.groups.clone()))
    });
    let Some(Some((windows, group, groups))) = moved else { return };
    windows.place(&group);

    SNAPSHOT.with(|cell| {
        let Ok(mut snapshot) = cell.try_borrow_mut() else { return };
        let Some(snapshot) = snapshot.as_mut() else { return };
        let origin = snapshot.grid.content_origin(&group);
        if snapshot.applied == Some(origin) {
            return;
        }
        snapshot.applied = Some(origin);
        DESKTOP.with(|desktop| {
            if let Ok(desktop) = desktop.try_borrow() {
                if let Some(desktop) = desktop.as_ref() {
                    _ = apply(desktop, &mut snapshot.items, snapshot.grid, &groups);
                }
            }
        });
    });
}

pub fn end_drag() {
    SNAPSHOT.with(|cell| {
        if let Ok(mut snapshot) = cell.try_borrow_mut() {
            *snapshot = None;
        }
    });
    let config = with_app(|app| {
        app.retries = 0;
        app.drag.take().map(|_| app.config.clone())
    });
    if let Some(Some(config)) = config {
        _ = config.save();
        layout_icons();
    }
}

fn reload_config() {
    let config = match Config::load_or_create() {
        Ok(config) => config,
        Err(e) => return show_error(&format!("Не удалось прочитать конфиг:\n{e}")),
    };
    with_app(|app| app.config = config);
    let handles = DESKTOP.with(|cell| {
        cell.try_borrow().ok().and_then(|d| d.as_ref().map(|d| (d.host, d.def_view)))
    });
    match handles {
        Some((host, def_view)) => {
            rebuild_windows(host, def_view);
            layout_icons();
        }
        None => attach(),
    }
}

fn open_config() {
    let path = HSTRING::from(format!("\"{}\"", Config::path().display()));
    unsafe { ShellExecuteW(None, w!("open"), w!("notepad.exe"), &path, None, SW_SHOWNORMAL) };
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() },
        ..Default::default()
    };
    for (dst, src) in data.szTip.iter_mut().zip("Corral".encode_utf16()) {
        *dst = src;
    }
    data
}

fn add_tray_icon(hwnd: HWND) {
    unsafe { _ = Shell_NotifyIconW(NIM_ADD, &tray_data(hwnd)) };
}

fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        _ = AppendMenuW(menu, MF_STRING, CMD_OPEN_CONFIG, w!("Открыть конфиг"));
        _ = AppendMenuW(menu, MF_STRING, CMD_RELOAD, w!("Перечитать конфиг"));
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, w!("Выход"));
        let mut cursor = POINT::default();
        _ = GetCursorPos(&mut cursor);
        _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, cursor.x, cursor.y, None, hwnd, None);
        _ = DestroyMenu(menu);
        match command.0 as usize {
            CMD_OPEN_CONFIG => open_config(),
            CMD_RELOAD => reload_config(),
            CMD_EXIT => _ = DestroyWindow(hwnd),
            _ => {}
        }
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_APP_DESKTOP_CHANGED => {
                with_app(|app| app.retries = 0);
                SetTimer(Some(hwnd), TIMER_LAYOUT, 400, None);
                LRESULT(0)
            }
            WM_TIMER => {
                _ = KillTimer(Some(hwnd), wparam.0);
                match wparam.0 {
                    TIMER_LAYOUT => layout_icons(),
                    TIMER_ATTACH => attach(),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_APP_TRAY => {
                let event = (lparam.0 & 0xFFFF) as u32;
                if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                    show_tray_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                _ = Shell_NotifyIconW(NIM_DELETE, &tray_data(hwnd));
                if let Some(windows) = with_app(|app| std::mem::take(&mut app.windows)) {
                    for w in windows {
                        w.destroy();
                    }
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                add_tray_icon(hwnd);
                detach();
                SetTimer(Some(hwnd), TIMER_ATTACH, 1500, None);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

