#![windows_subsystem = "windows"]

mod agent;
mod autostart;
mod config;
mod desktop;
mod group;
mod icon;
mod layout;
mod settings;
mod shelf;
mod shell;
mod watcher;

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WAIT_ABANDONED, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, ShellExecuteW, Shell_NotifyIconW};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    GetCursorPos, GetMessageW, IsDialogMessageW, KillTimer, MB_ICONERROR, MB_ICONWARNING, WM_LBUTTONUP,
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
const TIMER_LAYOUT: usize = 1;
const TIMER_ATTACH: usize = 2;
const TIMER_PEEK: usize = 3;
const TIMER_ANIMATE: usize = 4;
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
            groups.iter_mut().for_each(|g| g.collapsed = false);
        }
        groups
    }
}

struct Arranged {
    moved: usize,
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
        _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

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
            *cell.borrow_mut() = Some(App { main, instance, config, windows: Vec::new(), drag: None, retries: 0, agent: None, peek: None, animations: Vec::new() });
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
        .filter_map(|(index, group)| GroupWindows::create(index, group, host, def_view, instance, owned).ok())
        .collect();
    with_app(|app| app.windows = created);
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
                if owned {
                    show_members(arranged.members);
                }
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
    Ok(Arranged { moved, members })
}

fn show_members(members: Vec<Vec<(String, IShellItem)>>) {
    let names: Vec<String> = members.iter().flatten().map(|(name, _)| name.clone()).collect();
    let Some((agent, bodies)) =
        with_app(|app| (app.agent.take(), app.windows.iter().map(|w| (w.index, w.body)).collect::<Vec<_>>()))
    else {
        return;
    };
    if let Some(mut agent) = agent {
        agent.set_hidden(names);
        with_app(|app| app.agent = Some(agent));
    }
    for (index, list) in members.into_iter().enumerate() {
        if let Some(&(_, body)) = bodies.iter().find(|(i, _)| *i == index) {
            shelf::set_items(body, list);
        }
    }
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
            app.drag = Some(Drag { index, cursor, start: Frame::of(group), resize, moved: false });
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
    let grid = SNAPSHOT.with(|cell| cell.try_borrow().ok().and_then(|s| s.as_ref().map(|s| s.grid)));
    let moved = with_app(|app| {
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
        Some((windows, group, app.layout_groups()))
    });
    let Some(Some((windows, group, groups))) = moved else { return };
    windows.place(&group);

    SNAPSHOT.with(|cell| {
        let Ok(mut snapshot) = cell.try_borrow_mut() else { return };
        let Some(snapshot) = snapshot.as_mut() else { return };
        let frame = Frame::of(&group);
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
            layout_icons();
        }
        Some(Some((drag, _))) if drag.resize.is_none() => toggle_collapsed(drag.index),
        _ => {}
    }
}

fn toggle_collapsed(index: usize) {
    let changed = with_app(|app| {
        app.agent.as_ref()?;
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
    layout_icons();
}

pub fn hover_header(index: usize) {
    let opened = with_app(|app| {
        let collapsed = app.agent.is_some()
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
                LRESULT(0)
            }
            WM_TIMER => {
                _ = KillTimer(Some(hwnd), wparam.0);
                match wparam.0 {
                    TIMER_LAYOUT => layout_icons(),
                    TIMER_ATTACH => attach(),
                    TIMER_PEEK => check_peek(),
                    TIMER_ANIMATE => step_animations(),
                    _ => {}
                }
                LRESULT(0)
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

