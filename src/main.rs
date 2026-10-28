#![windows_subsystem = "windows"]

mod agent;
mod autostart;
mod composition;
mod config;
mod desktop;
mod dragging;
mod drop;
mod group;
mod icon;
mod layout;
mod portal;
mod rename;
mod settings;
mod shelf;
mod themes;
mod shell;
mod watcher;

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WAIT_ABANDONED, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, ShellExecuteW, Shell_NotifyIconW};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    GetCursorPos, GetMessageW, IDYES, WM_COPYDATA, MF_CHECKED, IsDialogMessageW, MB_ICONQUESTION, MB_YESNO, WindowFromPoint, KillTimer, MB_ICONERROR, MB_ICONWARNING, WM_LBUTTONUP,
    MF_SEPARATOR, MF_STRING, MSG,
    MessageBoxW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SW_SHOWNORMAL, SetForegroundWindow,
    SetTimer, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CONTEXTMENU, WM_DESTROY,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::Win32::UI::Shell::IShellItem;
use windows::core::{HSTRING, w};

use agent::Agent;
use config::{Appearance, Config, Group};
use desktop::{Desktop, DesktopItem};
use group::GroupWindows;
use layout::{Frame, Grid, Icon};

const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_DESKTOP_CHANGED: u32 = WM_APP + 2;
const WM_APP_PORTAL_CHANGED: u32 = WM_APP + 3;
const TIMER_LATE_LAYOUT: usize = 8;
const TIMER_PORTALS: usize = 9;
const LATE_LAYOUT_MS: u32 = 1500;
const TIMER_LAYOUT: usize = 1;
const TIMER_ATTACH: usize = 2;
const TIMER_PEEK: usize = 3;
const TIMER_ANIMATE: usize = 4;
const TIMER_DRAG_PREVIEW: usize = 5;
const TIMER_RENAME: usize = 7;
const MENU_RENAME: usize = 1;
const MENU_PLAQUE: usize = 2;
const MENU_NEW: usize = 3;
const MENU_REMOVE: usize = 4;
const MENU_SETTINGS: usize = 5;
const MENU_PORTAL: usize = 6;
const DRAG_PREVIEW_MS: u32 = 250;
const OPEN_DURATION: Duration = Duration::from_millis(180);
const CLOSE_DURATION: Duration = Duration::from_millis(140);
const DRAG_THRESHOLD: i32 = 4;
const CMD_OPEN_CONFIG: usize = 1;
const CMD_RELOAD: usize = 2;
const CMD_EXIT: usize = 3;
const CMD_SETTINGS: usize = 4;
const CMD_NEW_GROUP: usize = 5;
const INSTANCE_WAIT_MS: u32 = 3000;
const MAX_RETRIES: u32 = 3;
const AGENT_GRACE: Duration = Duration::from_secs(60);

struct App {
    main: HWND,
    instance: HINSTANCE,
    config: Config,
    windows: Vec<GroupWindows>,
    drag: Option<Drag>,
    retries: u32,
    agent: Option<Agent>,
    grid: Option<Grid>,
    menu_point: Option<POINT>,
    pending_rename: Option<String>,
    peek: Option<usize>,
    animations: Vec<Animation>,
}

struct Animation {
    index: usize,
    from: f32,
    to: f32,
    started: Instant,
}

impl Animation {
    fn progress(&self, now: Instant) -> f32 {
        let duration = if self.to > self.from { OPEN_DURATION } else { CLOSE_DURATION };
        (now.duration_since(self.started).as_secs_f32() / duration.as_secs_f32()).min(1.0)
    }

    fn fraction(&self, now: Instant) -> f32 {
        let eased = 1.0 - (1.0 - self.progress(now)).powi(3);
        self.from + (self.to - self.from) * eased
    }
}

impl App {
    fn layout_groups(&self) -> Vec<Group> {
        let mut groups = self.config.groups.clone();
        if self.agent.is_none() {
            groups.iter_mut().filter(|g| g.portal.is_none()).for_each(|g| g.collapsed = false);
        }
        groups
    }

    fn can_collapse(&self, index: usize) -> bool {
        self.agent.is_some() || self.config.groups.get(index).is_some_and(|g| g.portal.is_some())
    }
}

struct Arranged {
    moved: usize,
    grid: Grid,
    members: Vec<Vec<(String, IShellItem)>>,
}

struct Snapshot {
    items: Vec<DesktopItem>,
    grid: Grid,
    applied: Option<Frame>,
}

#[derive(Clone, Copy)]
struct Drag {
    index: usize,
    cursor: POINT,
    start: Frame,
    resize: Option<(bool, bool)>,
    moved: bool,
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

fn handles() -> Option<(HWND, HWND, HWND)> {
    DESKTOP.with(|cell| cell.try_borrow().ok()?.as_ref().map(|d| (d.host, d.def_view, d.list_view())))
}

fn main() {
    unsafe {
        _ = OleInitialize(None);

        let guard = CreateMutexW(None, true, w!("Local\\CorralSingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let Ok(handle) = &guard else { return };
            let waited = WaitForSingleObject(*handle, INSTANCE_WAIT_MS);
            if waited != WAIT_OBJECT_0 && waited != WAIT_ABANDONED {
                return;
            }
        }

        let config = match Config::load_or_create() {
            Ok(config) => config,
            Err(e) => return show_error(&format!("Не удалось прочитать конфиг:\n{e}")),
        };
        group::set_style(&config.appearance);

        let instance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        group::register_classes(instance);
        settings::register(instance);
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
            *cell.borrow_mut() = Some(App { main, instance, config, windows: Vec::new(), drag: None, retries: 0, agent: None, grid: None, menu_point: None, pending_rename: None, peek: None, animations: Vec::new() });
        });
        add_tray_icon(main);
        watcher::watch_desktop(main, WM_APP_DESKTOP_CHANGED);
        attach();

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if settings::window().is_some_and(|w| IsDialogMessageW(w, &msg).as_bool()) {
                continue;
            }
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
    let (host, def_view, list_view) = (desktop.host, desktop.def_view, desktop.list_view());
    let stored = DESKTOP.with(|cell| cell.try_borrow_mut().map(|mut d| *d = Some(desktop)).is_ok());
    if !stored {
        return schedule(TIMER_ATTACH, 200);
    }
    connect_agent(list_view);
    fit_groups();
    rebuild_windows(host, def_view);
    layout_icons();
}

fn fit_groups() {
    let Some(snapshot) = take_snapshot() else { return };
    let changed = with_app(|app| {
        let mut changed = false;
        for group in &mut app.config.groups {
            let center = POINT { x: group.x + group.width / 2, y: group.y + group::HEADER_HEIGHT / 2 };
            let fitted = layout::fit_group(Frame::of(group), Some(snapshot.grid), desktop::work_area(center));
            if fitted != Frame::of(group) {
                fitted.apply(group);
                changed = true;
            }
        }
        changed.then(|| app.config.clone())
    });
    if let Some(Some(config)) = changed {
        _ = config.save();
    }
}

fn connect_agent(list_view: HWND) {
    if agent::disabled_flag().exists() || with_app(|app| app.agent.is_some()) != Some(false) {
        return;
    }
    let agent = Agent::inject(list_view).ok();
    with_app(|app| app.agent = agent);
}

fn check_agent() {
    let lost = with_app(|app| app.agent.as_ref().is_some_and(|a| !a.alive())) == Some(true);
    if lost {
        with_app(|app| app.agent = None);
        if let Some((host, def_view, _)) = handles() {
            rebuild_windows(host, def_view);
        }
    }
}

pub fn config() -> Option<Config> {
    with_app(|app| app.config.clone())
}

fn save_config() {
    if let Some(config) = config() {
        _ = config.save();
    }
}

fn rebuild_and_layout() {
    if let Some((host, def_view, _)) = handles() {
        rebuild_windows(host, def_view);
        layout_icons();
    }
}

pub fn rename_group(index: usize, name: &str) {
    let renamed = with_app(|app| {
        let group = app.config.groups.get_mut(index)?;
        group.name = name.to_string();
        let group = group.clone();
        let windows = *app.windows.iter().find(|w| w.index == index)?;
        Some((windows, group, app.agent.is_some()))
    });
    if let Some(Some((windows, group, owned))) = renamed {
        if owned {
            windows.set_marker(&group, group.collapsed);
        } else {
            windows.set_title(&group.name);
        }
    }
    save_config();
}

pub fn set_rules(index: usize, rules: Vec<String>) {
    let changed = with_app(|app| {
        let group = app.config.groups.get_mut(index)?;
        (group.rules != rules).then(|| group.rules = rules)
    });
    if let Some(Some(())) = changed {
        save_config();
        layout_icons();
    }
}

pub fn set_modern_menu(enabled: bool) {
    with_app(|app| app.config.modern_menu = enabled);
    save_config();
}

pub fn modern_menu(index: usize, name: &str, at: POINT) -> bool {
    if with_app(|app| app.agent.is_some() && app.config.modern_menu) != Some(true) || portal_path(index).is_some() {
        return false;
    }
    let placed = DESKTOP.with(|cell| {
        let Ok(desktop) = cell.try_borrow() else { return false };
        let Some(desktop) = desktop.as_ref() else { return false };
        let Ok(items) = desktop.items() else { return false };
        let Some(item) = items.iter().find(|i| i.name == name) else { return false };
        desktop.position(&[(item, POINT { x: at.x - 38, y: at.y - 30 })]).is_ok()
    });
    if !placed {
        return false;
    }
    if let Some((_, _, list_view)) = handles() {
        let mut explorer = 0;
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(list_view, Some(&mut explorer));
            _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(explorer);
        }
    }
    with_app(|app| {
        app.menu_point = Some(at);
        app.agent.as_ref().map(|a| a.show_menu(name, at))
    });
    true
}

pub fn rename_file(old: &str, new: &str) {
    let renamed = DESKTOP.with(|cell| {
        let desktop = cell.try_borrow().ok()?;
        let desktop = desktop.as_ref()?;
        let items = desktop.items().ok()?;
        let item = items.iter().find(|i| i.name == old)?;
        Some(desktop.rename(item, new))
    });
    match renamed {
        Some(Ok(file)) => {
            with_app(|app| {
                for group in &mut app.config.groups {
                    for list in [&mut group.items, &mut group.exclude] {
                        for entry in list.iter_mut().filter(|e| e.eq_ignore_ascii_case(old)) {
                            *entry = file.clone();
                        }
                    }
                }
            });
            save_config();
            schedule(TIMER_LAYOUT, 200);
        }
        Some(Err(e)) => show_error(&format!("Не удалось переименовать «{old}»:\n{e}")),
        None => {}
    }
}

fn rename_request(name: &str) {
    let editing = DESKTOP.with(|cell| {
        let desktop = cell.try_borrow().ok()?;
        let desktop = desktop.as_ref()?;
        let items = desktop.items().ok()?;
        desktop.editing_name(items.iter().find(|i| i.name == name)?).ok()
    });
    let Some(editing) = editing else { return };
    let mut at = with_app(|app| app.menu_point).flatten().unwrap_or_default();
    if at == POINT::default() {
        unsafe { _ = GetCursorPos(&mut at) };
    }
    let rect = windows::Win32::Foundation::RECT { left: at.x - 90, top: at.y + 18, right: at.x + 90, bottom: at.y + 44 };
    rename::start_for(rename::Target::File(name.to_string()), rect, &editing);
}

fn agent_event(data: &windows::Win32::System::DataExchange::COPYDATASTRUCT) {
    let words = if data.lpData.is_null() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(data.lpData as *const u16, data.cbData as usize / 2) }
    };
    match data.dwData {
        corral_agent::protocol::EVT_RENAME => {
            if let Some(name) = corral_agent::protocol::decode_names(words).into_iter().next() {
                if let Some(main) = with_app(|app| app.main) {
                    with_app(|app| app.pending_rename = Some(name));
                    unsafe { SetTimer(Some(main), TIMER_RENAME, 10, None) };
                }
            }
        }
        corral_agent::protocol::EVT_MENU_DONE => schedule(TIMER_LAYOUT, 300),
        _ => {}
    }
}

pub fn drag_member(shelf: HWND, index: usize, entry: usize, name: &str, item: &IShellItem) {
    if portal_path(index).is_some() {
        drop::drag_item(shelf, item);
        return;
    }
    let icons = visible_icon_cells();
    let views = handles().map(|(_, def_view, list_view)| (def_view, list_view));
    let classify = |cursor: POINT, under: HWND| {
        if let Some(Some(group)) = with_app(|app| app.windows.iter().find(|w| w.owns(under)).map(|w| w.index)) {
            return dragging::Target::Group(group);
        }
        let on_desktop = views.is_some_and(|(def_view, list_view)| under == def_view || under == list_view);
        let on_icon = icons.iter().any(|r| cursor.x >= r.left && cursor.x < r.right && cursor.y >= r.top && cursor.y < r.bottom);
        if on_desktop && !on_icon { dragging::Target::Desktop } else { dragging::Target::Elsewhere }
    };
    match dragging::run(shelf, entry, classify) {
        dragging::Outcome::Group(target) if target != index => put_into_group(target, vec![name.to_string()]),
        dragging::Outcome::Desktop => dragged_out(index, name),
        dragging::Outcome::Handoff => {
            with_app(|app| app.agent.as_ref().map(|a| a.set_dragging(true)));
            let dropped = drop::drag_item(shelf, item);
            with_app(|app| app.agent.as_ref().map(|a| a.set_dragging(false)));
            if dropped {
                dragged_out(index, name);
            }
        }
        _ => {}
    }
}

fn visible_icon_cells() -> Vec<windows::Win32::Foundation::RECT> {
    let Some(groups) = with_app(|app| app.layout_groups()) else { return Vec::new() };
    DESKTOP.with(|cell| {
        let Ok(desktop) = cell.try_borrow() else { return Vec::new() };
        let Some(desktop) = desktop.as_ref() else { return Vec::new() };
        let (Ok(items), Ok(spacing)) = (desktop.items(), desktop.spacing()) else { return Vec::new() };
        items
            .iter()
            .filter(|i| config::group_of(&groups, &i.name).is_none())
            .map(|i| windows::Win32::Foundation::RECT {
                left: i.position.x,
                top: i.position.y,
                right: i.position.x + spacing.x,
                bottom: i.position.y + spacing.y,
            })
            .collect()
    })
}

fn dragged_out(index: usize, name: &str) {
    let mut cursor = POINT::default();
    unsafe { _ = GetCursorPos(&mut cursor) };
    let Some((_, def_view, list_view)) = handles() else { return };
    let under = unsafe { WindowFromPoint(cursor) };
    if under != list_view && under != def_view {
        return;
    }
    if !drop::desktop_dirs().iter().any(|dir| dir.join(name).exists()) {
        return;
    }
    let Some(groups) = with_app(|app| app.layout_groups()) else { return };
    let placed = DESKTOP.with(|cell| {
        let Ok(desktop) = cell.try_borrow() else { return false };
        let Some(desktop) = desktop.as_ref() else { return false };
        let Ok(items) = desktop.items() else { return false };
        let Ok(spacing) = desktop.spacing() else { return false };
        let on_icon = items.iter().any(|i| {
            config::group_of(&groups, &i.name).is_none()
                && (i.position.x..i.position.x + spacing.x).contains(&cursor.x)
                && (i.position.y..i.position.y + spacing.y).contains(&cursor.y)
        });
        if on_icon {
            return false;
        }
        let grid = Grid::detect(items.iter().map(|i| i.position), spacing);
        let Some(item) = items.iter().find(|i| i.name == name) else { return false };
        let taken: Vec<POINT> = items.iter().filter(|i| i.name != name).map(|i| i.position).collect();
        let area = desktop::work_area(cursor);
        if let Some(target) = layout::nearest_free_cell(cursor, grid, &taken, &groups, area) {
            _ = desktop.position(&[(item, target)]);
        }
        true
    });
    if !placed {
        return;
    }
    with_app(|app| {
        app.config.take_out(index, name);
        app.retries = 0;
    });
    save_config();
    layout_icons();
}

pub fn put_into_group(index: usize, names: Vec<String>) {
    let changed = with_app(|app| {
        if index >= app.config.groups.len() {
            return false;
        }
        for name in &names {
            app.config.put(index, name);
        }
        true
    });
    if changed == Some(true) {
        save_config();
        with_app(|app| app.retries = 0);
        schedule(TIMER_LAYOUT, 50);
    }
}

pub fn set_collapsed(index: usize, collapsed: bool) {
    let current = with_app(|app| app.config.groups.get(index).map(|g| g.collapsed)).flatten();
    if current.is_some_and(|c| c != collapsed) {
        toggle_collapsed(index);
    }
}

pub fn add_group() -> Option<usize> {
    let area = desktop::work_area(POINT::default());
    let index = with_app(|app| {
        let (width, height) = (4 * 76, 3 * 100);
        let x = (area.left + area.right - width) / 2;
        let y = (area.top + area.bottom - height) / 2;
        let name = format!("Группа {}", app.config.groups.len() + 1);
        app.config.groups.push(Group { name, x, y, width, height, ..Group::empty() });
        app.config.groups.len() - 1
    })?;
    fit_groups();
    save_config();
    rebuild_and_layout();
    Some(index)
}

pub fn remove_group(index: usize) {
    let removed = with_app(|app| (index < app.config.groups.len()).then(|| app.config.groups.remove(index)));
    if let Some(Some(_)) = removed {
        save_config();
        rebuild_and_layout();
    }
}

pub fn set_appearance(appearance: Appearance) {
    group::set_style(&appearance);
    let windows = with_app(|app| {
        app.config.appearance = appearance;
        app.windows.clone()
    });
    for windows in windows.unwrap_or_default() {
        windows.restyle();
        shelf::render(windows.body);
    }
    save_config();
}

pub fn agent_enabled() -> bool {
    !agent::disabled_flag().exists()
}

pub fn set_agent(enabled: bool) {
    if enabled != agent_enabled() {
        toggle_agent();
    }
}

pub fn set_autostart(enabled: bool) {
    if enabled != autostart::enabled() {
        if let Some(main) = with_app(|app| app.main) {
            toggle_autostart(main);
        }
    }
}

fn toggle_agent() {
    let flag = agent::disabled_flag();
    if flag.exists() {
        _ = std::fs::remove_file(&flag);
        if let Some((_, _, list_view)) = handles() {
            connect_agent(list_view);
        }
    } else {
        _ = std::fs::write(&flag, "");
        with_app(|app| app.agent = None);
    }
    if let Some((host, def_view, _)) = handles() {
        rebuild_windows(host, def_view);
        layout_icons();
    }
}

fn detach() {
    DESKTOP.with(|cell| {
        if let Ok(mut desktop) = cell.try_borrow_mut() {
            *desktop = None;
        }
    });
}

fn rebuild_windows(host: HWND, def_view: HWND) {
    let Some((old, groups, instance, owned)) = with_app(|app| {
        app.peek = None;
        app.animations.clear();
        (std::mem::take(&mut app.windows), app.config.groups.clone(), app.instance, app.agent.is_some())
    }) else {
        return;
    };
    for windows in old {
        windows.destroy();
    }
    let created: Vec<GroupWindows> = groups
        .iter()
        .enumerate()
        .filter_map(|(index, group)| {
            let owned = owned || group.portal.is_some();
            GroupWindows::create(index, group, host, def_view, instance, owned).ok()
        })
        .collect();
    let main = with_app(|app| {
        app.windows = created;
        app.main
    });
    if let Some(main) = main {
        for path in groups.iter().filter_map(|g| g.portal.as_deref()) {
            portal::watch(path, main, WM_APP_PORTAL_CHANGED);
        }
    }
}

fn refresh_portals() {
    let Some(portals) = with_app(|app| {
        app.windows
            .iter()
            .filter_map(|w| Some((w.body, app.config.groups.get(w.index)?.portal.clone()?)))
            .collect::<Vec<_>>()
    }) else {
        return;
    };
    for (body, path) in portals {
        shelf::set_items(body, portal::items(&path));
    }
}

pub fn portal_path(index: usize) -> Option<String> {
    with_app(|app| app.config.groups.get(index)?.portal.clone()).flatten()
}

pub fn add_portal(path: String) -> Option<usize> {
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let index = add_group()?;
    with_app(|app| {
        if let Some(group) = app.config.groups.get_mut(index) {
            group.name = name;
            group.portal = Some(path);
        }
    });
    save_config();
    rebuild_and_layout();
    Some(index)
}

pub fn set_portal(index: usize, path: Option<String>) {
    let changed = with_app(|app| {
        let group = app.config.groups.get_mut(index)?;
        (group.portal != path).then(|| group.portal = path)
    });
    if let Some(Some(())) = changed {
        save_config();
        rebuild_and_layout();
    }
}

fn pick_portal() {
    let owner = settings::window();
    if let Some(path) = portal::pick_folder(owner) {
        add_portal(path);
        settings::refresh();
    }
}

fn layout_icons() {
    check_agent();
    let Some((groups, owned)) = with_app(|app| (app.layout_groups(), app.agent.is_some())) else {
        return schedule(TIMER_LAYOUT, 200);
    };
    let result = DESKTOP.with(|cell| match cell.try_borrow() {
        Ok(desktop) => desktop.as_ref().map(|d| arrange(d, &groups, owned).map(Some)),
        Err(_) => Some(Ok(None)),
    });
    let result = result.map(|r| {
        r.map(|arranged| {
            arranged.map(|arranged| {
                with_app(|app| app.grid = Some(arranged.grid));
                show_members(arranged.members, owned);
                arranged.moved
            })
        })
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

fn arrange(desktop: &Desktop, groups: &[Group], owned: bool) -> windows::core::Result<Arranged> {
    let mut items = desktop.items()?;
    let dirs = drop::desktop_dirs();
    items.retain(|i| i.name.starts_with("::") || dirs.iter().any(|d| d.join(&i.name).exists()));
    let grid = Grid::detect(items.iter().map(|i| i.position), desktop.spacing()?);
    let moved = apply(desktop, &mut items, grid, groups)?;
    let mut members = vec![Vec::new(); groups.len()];
    if owned {
        shelf::set_spacing(grid.spacing);
        for item in &items {
            let Some(list) = config::group_of(groups, &item.name).and_then(|g| members.get_mut(g)) else { continue };
            if let Ok(shell_item) = desktop.shell_item(item) {
                list.push((item.name.clone(), shell_item));
            }
        }
        for list in &mut members {
            list.sort_by_key(|(name, _)| name.to_lowercase());
        }
    }
    Ok(Arranged { moved, grid, members })
}

fn show_members(members: Vec<Vec<(String, IShellItem)>>, owned: bool) {
    let names: Vec<String> = members.iter().flatten().map(|(name, _)| name.clone()).collect();
    let Some((agent, bodies)) = with_app(|app| {
        let bodies: Vec<(usize, HWND, Option<String>)> = app
            .windows
            .iter()
            .map(|w| (w.index, w.body, app.config.groups.get(w.index).and_then(|g| g.portal.clone())))
            .collect();
        (app.agent.take(), bodies)
    }) else {
        return;
    };
    if let Some(mut agent) = agent {
        agent.set_hidden(names);
        with_app(|app| app.agent = Some(agent));
    }
    let mut members = members;
    for (index, body, portal) in bodies {
        match portal {
            Some(path) => shelf::set_items(body, portal::items(&path)),
            None if owned => shelf::set_items(body, members.get_mut(index).map(std::mem::take).unwrap_or_default()),
            None => {}
        }
    }
}

pub fn is_portal(index: usize) -> bool {
    portal_path(index).is_some()
}

pub fn take_out(index: usize, name: &str) {
    let config = with_app(|app| {
        app.config.take_out(index, name);
        app.config.clone()
    });
    if let Some(config) = config {
        _ = config.save();
        layout_icons();
    }
}

fn apply(desktop: &Desktop, items: &mut [DesktopItem], grid: Grid, groups: &[Group]) -> windows::core::Result<usize> {
    let icons: Vec<Icon> = items
        .iter()
        .map(|i| Icon { name: &i.name, position: i.position, group: config::group_of(groups, &i.name) })
        .collect();
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
    begin(index, None);
}

pub fn begin_resize(index: usize, right: bool, bottom: bool) {
    begin(index, Some((right, bottom)));
}

fn begin(index: usize, resize: Option<(bool, bool)>) {
    let mut cursor = POINT::default();
    unsafe { _ = GetCursorPos(&mut cursor) };
    with_app(|app| {
        if let Some(group) = app.config.groups.get(index) {
            let area = desktop::work_area(POINT { x: group.x + group.width / 2, y: group.y + group::HEADER_HEIGHT / 2 });
            let start = layout::trim_group(Frame::of(group), app.grid, area);
            app.drag = Some(Drag { index, cursor, start, resize, moved: false });
        }
    });
    SNAPSHOT.with(|cell| {
        if let Ok(mut current) = cell.try_borrow_mut() {
            *current = None;
        }
    });
}

pub fn drag_to_cursor() {
    let mut cursor = POINT::default();
    unsafe { _ = GetCursorPos(&mut cursor) };
    let moved = with_app(|app| {
        let grid = app.grid;
        let drag = app.drag.as_mut()?;
        let (dx, dy) = (cursor.x - drag.cursor.x, cursor.y - drag.cursor.y);
        if !drag.moved && dx.abs() < DRAG_THRESHOLD && dy.abs() < DRAG_THRESHOLD {
            return None;
        }
        drag.moved = true;
        let (index, start) = (drag.index, drag.start);
        let fitted = match drag.resize {
            None => {
                let target = Frame { x: start.x + dx, y: start.y + dy, ..start };
                layout::fit_group(target, grid, desktop::work_area(cursor))
            }
            Some((right, bottom)) => {
                let width = start.width + if right { dx } else { 0 };
                let height = start.height + if bottom { dy } else { 0 };
                let area = desktop::work_area(POINT { x: start.x, y: start.y });
                layout::resize_group(Frame { width, height, ..start }, grid, area)
            }
        };
        let group = app.config.groups.get_mut(index)?;
        if Frame::of(group) == fitted {
            return None;
        }
        fitted.apply(group);
        let group = group.clone();
        let windows = *app.windows.iter().find(|w| w.index == index)?;
        Some((windows, group))
    });
    let Some(Some((windows, group))) = moved else { return };
    windows.place(&group);
    schedule(TIMER_DRAG_PREVIEW, DRAG_PREVIEW_MS);
}

fn drag_preview() {
    let Some(Some((index, groups))) = with_app(|app| {
        let drag = app.drag.as_ref().filter(|d| d.moved)?;
        Some((drag.index, app.layout_groups()))
    }) else {
        return;
    };
    let Some(frame) = groups.get(index).map(Frame::of) else { return };
    let missing = SNAPSHOT.with(|cell| cell.try_borrow().map(|s| s.is_none()).unwrap_or(false));
    if missing {
        let snapshot = take_snapshot();
        SNAPSHOT.with(|cell| {
            if let Ok(mut current) = cell.try_borrow_mut() {
                *current = snapshot;
            }
        });
    }
    let still_dragging = with_app(|app| app.drag.as_ref().is_some_and(|d| d.index == index)) == Some(true);
    if !still_dragging {
        return;
    }
    SNAPSHOT.with(|cell| {
        let Ok(mut snapshot) = cell.try_borrow_mut() else { return };
        let Some(snapshot) = snapshot.as_mut() else { return };
        if snapshot.applied == Some(frame) {
            return;
        }
        snapshot.applied = Some(frame);
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
    if let Some(main) = with_app(|app| app.main) {
        unsafe { _ = KillTimer(Some(main), TIMER_DRAG_PREVIEW) };
    }
    SNAPSHOT.with(|cell| {
        if let Ok(mut snapshot) = cell.try_borrow_mut() {
            *snapshot = None;
        }
    });
    let finished = with_app(|app| {
        app.retries = 0;
        app.drag.take().map(|drag| (drag, app.config.clone()))
    });
    match finished {
        Some(Some((drag, config))) if drag.moved => {
            _ = config.save();
            schedule(TIMER_LAYOUT, 0);
        }
        _ => {}
    }
}

pub fn header_double_click(index: usize) {
    if let Some(Some(group)) = with_app(|app| app.config.groups.get(index).cloned()) {
        start_rename(index, &group);
    }
}

fn start_rename(index: usize, group: &Group) {
    let header = windows::Win32::Foundation::RECT {
        left: group.x,
        top: group.y,
        right: group.x + group.width,
        bottom: group.y + group::HEADER_HEIGHT,
    };
    rename::start(index, header, &group.name);
}

pub fn header_menu(index: usize) {
    let Some(Some((group, owned))) = with_app(|app| Some((app.config.groups.get(index)?.clone(), app.can_collapse(index))))
    else {
        return;
    };
    let Some(main) = with_app(|app| app.main) else { return };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        _ = AppendMenuW(menu, MF_STRING, MENU_RENAME, w!("Переименовать"));
        if owned {
            let flags = if group.collapsed { MF_STRING | MF_CHECKED } else { MF_STRING };
            _ = AppendMenuW(menu, flags, MENU_PLAQUE, w!("Режим плашки"));
        }
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        _ = AppendMenuW(menu, MF_STRING, MENU_NEW, w!("Новая группа"));
        _ = AppendMenuW(menu, MF_STRING, MENU_PORTAL, w!("Новый портал в папку…"));
        _ = AppendMenuW(menu, MF_STRING, MENU_REMOVE, w!("Удалить группу…"));
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS, w!("Настройки…"));
        let mut cursor = POINT::default();
        _ = GetCursorPos(&mut cursor);
        _ = SetForegroundWindow(main);
        let command = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, cursor.x, cursor.y, None, main, None);
        _ = DestroyMenu(menu);
        match command.0 as usize {
            MENU_RENAME => start_rename(index, &group),
            MENU_PLAQUE => toggle_collapsed(index),
            MENU_NEW => {
                add_group();
                settings::refresh();
            }
            MENU_REMOVE => confirm_remove(index, &group.name),
            MENU_SETTINGS => open_settings(),
            MENU_PORTAL => pick_portal(),
            _ => {}
        }
    }
}

fn confirm_remove(index: usize, name: &str) {
    let question = format!("Удалить группу «{name}»?\n\nФайлы на рабочем столе останутся на месте.");
    let answer = unsafe { MessageBoxW(None, &HSTRING::from(question), w!("Corral"), MB_YESNO | MB_ICONQUESTION) };
    if answer == IDYES {
        remove_group(index);
        settings::refresh();
    }
}

fn toggle_collapsed(index: usize) {
    let changed = with_app(|app| {
        if !app.can_collapse(index) {
            return None;
        }
        let group = app.config.groups.get_mut(index)?;
        group.collapsed = !group.collapsed;
        let group = group.clone();
        let windows = *app.windows.iter().find(|w| w.index == index)?;
        app.peek = group.collapsed.then_some(index);
        Some((windows, group, app.config.clone()))
    });
    let Some(Some((windows, group, config))) = changed else { return };
    windows.set_marker(&group, group.collapsed);
    animate(index, !group.collapsed);
    settings::refresh();
    if group.collapsed {
        schedule(TIMER_PEEK, 150);
    }
    _ = config.save();
    schedule(TIMER_LAYOUT, 0);
}

pub fn hover_header(index: usize) {
    let opened = with_app(|app| {
        let collapsed = app.can_collapse(index)
            && app.config.appearance.hover_open
            && app.config.groups.get(index).is_some_and(|g| g.collapsed);
        if !collapsed || app.drag.is_some() || app.peek == Some(index) {
            return None;
        }
        Some(app.peek.replace(index))
    });
    let Some(Some(previous)) = opened else { return };
    if let Some(previous) = previous {
        animate(previous, false);
    }
    animate(index, true);
    schedule(TIMER_PEEK, 150);
}

fn animate(index: usize, open: bool) {
    let now = Instant::now();
    let started = with_app(|app| {
        let windows = *app.windows.iter().find(|w| w.index == index)?;
        if !app.config.appearance.animate {
            app.animations.retain(|a| a.index != index);
            return Some((windows, app.config.groups.get(index)?.clone(), None));
        }
        let from = match app.animations.iter().position(|a| a.index == index) {
            Some(i) => app.animations.remove(i).fraction(now),
            None if windows.is_open() => 1.0,
            None => 0.0,
        };
        app.animations.push(Animation { index, from, to: if open { 1.0 } else { 0.0 }, started: now });
        Some((windows, app.config.groups.get(index)?.clone(), Some(from)))
    });
    let Some(Some((windows, group, from))) = started else { return };
    let Some(from) = from else {
        windows.place(&group);
        windows.set_open(open);
        return;
    };
    if open {
        windows.reveal(&group, from);
        windows.set_open(true);
    }
    schedule(TIMER_ANIMATE, 10);
}

fn step_animations() {
    let now = Instant::now();
    let frames = with_app(|app| {
        let (windows, groups) = (&app.windows, &app.config.groups);
        let mut frames = Vec::new();
        app.animations.retain(|a| {
            let done = a.progress(now) >= 1.0;
            if let (Some(w), Some(g)) = (windows.iter().find(|w| w.index == a.index), groups.get(a.index)) {
                frames.push((*w, g.clone(), a.fraction(now), done && a.to == 0.0));
            }
            !done
        });
        (frames, !app.animations.is_empty())
    });
    let Some((frames, more)) = frames else { return };
    for (windows, group, fraction, hide) in frames {
        if hide {
            windows.set_open(false);
            windows.place(&group);
        } else {
            windows.reveal(&group, fraction);
        }
    }
    if more {
        schedule(TIMER_ANIMATE, 10);
    }
}

fn check_peek() {
    let mut cursor = POINT::default();
    unsafe { _ = GetCursorPos(&mut cursor) };
    let busy = shell::menu_open();
    let closed = with_app(|app| {
        let index = app.peek?;
        let group = app.config.groups.get(index)?;
        let inside = cursor.x >= group.x
            && cursor.x < group.x + group.width
            && cursor.y >= group.y
            && cursor.y < group.y + group.height;
        if busy || inside || app.drag.is_some() {
            return Some(None);
        }
        app.peek = None;
        Some(Some(index))
    });
    match closed {
        Some(Some(Some(index))) => animate(index, false),
        Some(Some(None)) => schedule(TIMER_PEEK, 150),
        _ => {}
    }
}

fn reload_config() {
    let config = match Config::load_or_create() {
        Ok(config) => config,
        Err(e) => return show_error(&format!("Не удалось прочитать конфиг:\n{e}")),
    };
    group::set_style(&config.appearance);
    with_app(|app| app.config = config);
    settings::refresh();
    match handles() {
        Some((host, def_view, _)) => {
            fit_groups();
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
        hIcon: icon::tray_icon(),
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

fn toggle_autostart(main: HWND) {
    if autostart::enabled() {
        if let Err(e) = autostart::disable() {
            show_error(&format!("Не удалось убрать автозапуск:\n{e}"));
        }
        return;
    }
    if let Err(e) = autostart::enable() {
        return show_error(&format!("Не удалось включить автозапуск:\n{e}"));
    }
    if !autostart::is_installed_copy() && autostart::run_task().is_ok() {
        unsafe { _ = DestroyWindow(main) };
    }
}

fn open_settings() {
    if let Some(instance) = with_app(|app| app.instance) {
        settings::open(instance);
    }
}

fn show_tray_menu(hwnd: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        _ = AppendMenuW(menu, MF_STRING, CMD_SETTINGS, w!("Настройки…"));
        _ = AppendMenuW(menu, MF_STRING, CMD_NEW_GROUP, w!("Новая группа"));
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
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
            CMD_SETTINGS => open_settings(),
            CMD_NEW_GROUP => {
                add_group();
                settings::refresh();
            }
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
                SetTimer(Some(hwnd), TIMER_LATE_LAYOUT, LATE_LAYOUT_MS, None);
                LRESULT(0)
            }
            WM_APP_PORTAL_CHANGED => {
                SetTimer(Some(hwnd), TIMER_PORTALS, 250, None);
                LRESULT(0)
            }
            WM_TIMER => {
                _ = KillTimer(Some(hwnd), wparam.0);
                match wparam.0 {
                    TIMER_LAYOUT => layout_icons(),
                    TIMER_ATTACH => attach(),
                    TIMER_PEEK => check_peek(),
                    TIMER_ANIMATE => step_animations(),
                    TIMER_DRAG_PREVIEW => drag_preview(),
                    TIMER_LATE_LAYOUT => layout_icons(),
                    TIMER_PORTALS => refresh_portals(),
                    TIMER_RENAME => {
                        if let Some(Some(name)) = with_app(|app| app.pending_rename.take()) {
                            rename_request(&name);
                        }
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_COPYDATA => {
                agent_event(&*(lparam.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT));
                LRESULT(1)
            }
            WM_APP_TRAY => {
                let event = (lparam.0 & 0xFFFF) as u32;
                if event == WM_LBUTTONUP {
                    open_settings();
                }
                if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                    show_tray_menu(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                if let Some(settings) = settings::window() {
                    _ = DestroyWindow(settings);
                }
                _ = Shell_NotifyIconW(NIM_DELETE, &tray_data(hwnd));
                with_app(|app| app.agent = None);
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
                let crashed = with_app(|app| app.agent.take().is_some_and(|a| a.started.elapsed() < AGENT_GRACE));
                if crashed == Some(true) {
                    _ = std::fs::write(agent::disabled_flag(), "");
                    MessageBoxW(
                        None,
                        w!("Проводник перезапустился вскоре после подключения агента, поэтому агент отключён.\n\nИконки групп снова видны. Включить агент: меню в трее → «Прятать иконки в группах»."),
                        w!("Corral"),
                        MB_ICONWARNING,
                    );
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

