use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_BTNFACE, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DeleteObject, FW_NORMAL, HBRUSH, HFONT, InvalidateRect, OUT_DEFAULT_PRECIS,
};
use windows::Win32::UI::Controls::Dialogs::{CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, TBM_SETPOS, TBM_SETRANGE,
    TRACKBAR_CLASSW,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BN_CLICKED, BS_AUTOCHECKBOX, BS_GROUPBOX, BS_PUSHBUTTON, CreateWindowExW,
    DefWindowProcW, DestroyWindow, EN_CHANGE, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_WANTRETURN,
    GetDlgItem, GetWindowTextLengthW, GetWindowTextW, HMENU, ICON_BIG, ICON_SMALL, IDYES, KillTimer, LB_ADDSTRING,
    LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL, LBN_SELCHANGE, LBS_NOTIFY, MB_ICONQUESTION, MB_YESNO, MessageBoxW,
    RegisterClassW, SW_SHOWNORMAL, SWP_NOMOVE, SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetTimer,
    SetWindowPos, SetWindowTextW, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_HSCROLL, WM_SETFONT, WM_SETICON, WM_TIMER, WNDCLASSW, WS_BORDER, WS_CAPTION,
    WS_CHILD, WS_EX_CLIENTEDGE, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::config::{Appearance, format_color, parse_color};

const ID_LIST: i32 = 100;
const ID_ADD: i32 = 101;
const ID_REMOVE: i32 = 102;
const ID_NAME: i32 = 103;
const ID_RULES: i32 = 104;
const ID_COLLAPSED: i32 = 105;
const ID_HEADER_COLOR: i32 = 110;
const ID_BODY_COLOR: i32 = 111;
const ID_TITLE_COLOR: i32 = 112;
const ID_HEADER_SWATCH: i32 = 113;
const ID_BODY_SWATCH: i32 = 114;
const ID_TITLE_SWATCH: i32 = 115;
const ID_HEADER_OPACITY: i32 = 116;
const ID_BODY_OPACITY: i32 = 117;
const ID_ANIMATE: i32 = 118;
const ID_HOVER: i32 = 119;
const ID_AGENT: i32 = 120;
const ID_AUTOSTART: i32 = 121;
const ID_BORDER_COLOR: i32 = 122;
const ID_BORDER_SWATCH: i32 = 123;
const ID_ROUNDED: i32 = 124;
const ID_LABEL_COLOR: i32 = 125;
const ID_LABEL_SWATCH: i32 = 126;
const ID_THEME: i32 = 127;

const CBS_DROPDOWNLIST: u32 = 0x0003;
const CB_ADDSTRING: u32 = 0x0143;
const CB_GETCURSEL: u32 = 0x0147;
const CB_SETCURSEL: u32 = 0x014E;
const CBN_SELCHANGE: u32 = 1;
const CUSTOM_THEME: &str = "Своя тема";

const TBM_GETPOS: u32 = 0x0400;
const TIMER_RULES: usize = 1;
const RULES_DELAY_MS: u32 = 600;
const WIDTH: i32 = 680;
const HEIGHT: i32 = 616;
const GROUP_FIELDS: [i32; 4] = [ID_REMOVE, ID_NAME, ID_RULES, ID_COLLAPSED];

struct Settings {
    hwnd: HWND,
    selected: Option<usize>,
    loading: bool,
    font: HFONT,
    swatches: [(i32, HBRUSH); 5],
    custom: [COLORREF; 16],
}

thread_local! {
    static SETTINGS: RefCell<Option<Settings>> = const { RefCell::new(None) };
}

fn with_settings<R>(f: impl FnOnce(&mut Settings) -> R) -> Option<R> {
    SETTINGS.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

pub fn window() -> Option<HWND> {
    with_settings(|s| s.hwnd)
}

pub fn register(instance: HINSTANCE) {
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES | ICC_STANDARD_CLASSES,
        };
        _ = InitCommonControlsEx(&controls);
        let class = WNDCLASSW {
            lpfnWndProc: Some(settings_proc),
            hInstance: instance,
            lpszClassName: w!("CorralSettings"),
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as _),
            hCursor: windows::Win32::UI::WindowsAndMessaging::LoadCursorW(
                None,
                windows::Win32::UI::WindowsAndMessaging::IDC_ARROW,
            )
            .unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
    }
}

pub fn open(instance: HINSTANCE) {
    if let Some(hwnd) = window() {
        unsafe {
            _ = ShowWindow(hwnd, SW_SHOWNORMAL);
            _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    unsafe {
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE::default(), w!("CorralSettings"), w!("Corral — настройки"), style,
            200, 200, WIDTH, HEIGHT, None, None, Some(instance), None,
        ) else {
            return;
        };
        let dpi = GetDpiForWindow(hwnd).max(96);
        let scale = |v: i32| v * dpi as i32 / 96;
        let mut frame = RECT { left: 0, top: 0, right: scale(WIDTH), bottom: scale(HEIGHT) };
        _ = AdjustWindowRectExForDpi(&mut frame, style, false, WINDOW_EX_STYLE::default(), dpi);
        _ = SetWindowPos(hwnd, None, 0, 0, frame.right - frame.left, frame.bottom - frame.top, SWP_NOMOVE | SWP_NOZORDER);

        let font = CreateFontW(
            -scale(12), 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe UI"),
        );
        let icon = crate::icon::app_icon();
        SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(icon.0 as isize)));
        SendMessageW(hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(LPARAM(icon.0 as isize)));

        let add = |class: PCWSTR, text: &str, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE, id: i32, r: [i32; 4]| {
            let child = CreateWindowExW(
                ex, class, &HSTRING::from(text), WS_CHILD | WS_VISIBLE | style,
                scale(r[0]), scale(r[1]), scale(r[2]), scale(r[3]),
                Some(hwnd), Some(HMENU(id as isize as _)), Some(instance), None,
            );
            if let Ok(child) = child {
                SendMessageW(child, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
        };
        let none = WINDOW_EX_STYLE::default();
        let plain = WINDOW_STYLE::default();
        let button = WINDOW_STYLE(BS_PUSHBUTTON as u32) | WS_TABSTOP;
        let check = WINDOW_STYLE(BS_AUTOCHECKBOX as u32) | WS_TABSTOP;

        add(w!("STATIC"), "Группы", plain, none, -1, [16, 14, 200, 18]);
        add(w!("LISTBOX"), "", WINDOW_STYLE(LBS_NOTIFY as u32) | WS_VSCROLL | WS_BORDER | WS_TABSTOP, none, ID_LIST, [16, 34, 200, 380]);
        add(w!("BUTTON"), "Добавить", button, none, ID_ADD, [16, 420, 96, 28]);
        add(w!("BUTTON"), "Удалить", button, none, ID_REMOVE, [120, 420, 96, 28]);

        add(w!("STATIC"), "Название", plain, none, -1, [236, 14, 428, 18]);
        add(w!("EDIT"), "", WINDOW_STYLE(ES_AUTOHSCROLL as u32) | WS_TABSTOP, WS_EX_CLIENTEDGE, ID_NAME, [236, 34, 428, 24]);
        add(w!("STATIC"), "Правила: маски файлов, по одной на строку (например *.pdf)", plain, none, -1, [236, 68, 428, 18]);
        let multiline = WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32) | WS_VSCROLL | WS_TABSTOP;
        add(w!("EDIT"), "", multiline, WS_EX_CLIENTEDGE, ID_RULES, [236, 88, 428, 90]);
        add(w!("BUTTON"), "Режим плашки (только заголовок, раскрывается при наведении)", check, none, ID_COLLAPSED, [236, 186, 428, 22]);

        add(w!("BUTTON"), "Оформление", WINDOW_STYLE(BS_GROUPBOX as u32), none, -1, [236, 216, 428, 320]);
        add(w!("STATIC"), "Тема", plain, none, -1, [252, 241, 56, 18]);
        let combo = WINDOW_STYLE(CBS_DROPDOWNLIST) | WS_VSCROLL | WS_TABSTOP;
        add(w!("COMBOBOX"), "", combo, none, ID_THEME, [310, 236, 340, 320]);
        for (row, (id, swatch, text)) in [
            (ID_HEADER_COLOR, ID_HEADER_SWATCH, "Цвет шапки…"),
            (ID_BODY_COLOR, ID_BODY_SWATCH, "Цвет тела…"),
            (ID_TITLE_COLOR, ID_TITLE_SWATCH, "Цвет заголовка…"),
            (ID_BORDER_COLOR, ID_BORDER_SWATCH, "Цвет рамки…"),
            (ID_LABEL_COLOR, ID_LABEL_SWATCH, "Цвет подписей…"),
        ]
        .into_iter()
        .enumerate()
        {
            let y = 274 + row as i32 * 32;
            add(w!("BUTTON"), text, button, none, id, [252, y, 130, 26]);
            add(w!("STATIC"), "", WS_BORDER, none, swatch, [390, y + 2, 36, 22]);
        }
        let slider = WS_TABSTOP;
        add(w!("STATIC"), "Прозрачность шапки", plain, none, -1, [450, 276, 200, 18]);
        add(TRACKBAR_CLASSW, "", slider, none, ID_HEADER_OPACITY, [444, 294, 212, 28]);
        add(w!("STATIC"), "Прозрачность тела", plain, none, -1, [450, 330, 200, 18]);
        add(TRACKBAR_CLASSW, "", slider, none, ID_BODY_OPACITY, [444, 348, 212, 28]);
        add(w!("BUTTON"), "Анимация раскрытия", check, none, ID_ANIMATE, [252, 440, 300, 22]);
        add(w!("BUTTON"), "Раскрывать плашки при наведении", check, none, ID_HOVER, [252, 468, 300, 22]);
        add(w!("BUTTON"), "Скруглённые углы", check, none, ID_ROUNDED, [252, 496, 300, 22]);

        add(w!("BUTTON"), "Прятать иконки в группах (агент в Проводнике)", check, none, ID_AGENT, [236, 548, 428, 22]);
        add(w!("BUTTON"), "Запускать при входе в Windows", check, none, ID_AUTOSTART, [236, 576, 428, 22]);

        if let Ok(themes) = GetDlgItem(Some(hwnd), ID_THEME) {
            let names = crate::themes::THEMES.iter().map(|t| t.name).chain([CUSTOM_THEME]);
            for name in names {
                let wide = HSTRING::from(name);
                SendMessageW(themes, CB_ADDSTRING, None, Some(LPARAM(wide.as_ptr() as isize)));
            }
        }

        for id in [ID_HEADER_OPACITY, ID_BODY_OPACITY] {
            if let Ok(slider) = GetDlgItem(Some(hwnd), id) {
                SendMessageW(slider, TBM_SETRANGE, Some(WPARAM(1)), Some(LPARAM((255 << 16) | 20)));
            }
        }

        SETTINGS.with(|cell| {
            *cell.borrow_mut() = Some(Settings {
                hwnd,
                selected: None,
                loading: false,
                font,
                swatches: [
                    (ID_HEADER_SWATCH, HBRUSH::default()),
                    (ID_BODY_SWATCH, HBRUSH::default()),
                    (ID_TITLE_SWATCH, HBRUSH::default()),
                    (ID_BORDER_SWATCH, HBRUSH::default()),
                    (ID_LABEL_SWATCH, HBRUSH::default()),
                ],
                custom: [COLORREF(0x00FFFFFF); 16],
            })
        });
        refresh();
        _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        _ = SetForegroundWindow(hwnd);
    }
}

fn item(hwnd: HWND, id: i32) -> Option<HWND> {
    unsafe { GetDlgItem(Some(hwnd), id).ok() }
}

fn set_text(hwnd: HWND, id: i32, text: &str) {
    if let Some(control) = item(hwnd, id) {
        unsafe { _ = SetWindowTextW(control, &HSTRING::from(text)) };
    }
}

fn text(hwnd: HWND, id: i32) -> String {
    let Some(control) = item(hwnd, id) else { return String::new() };
    unsafe {
        let mut buf = vec![0u16; GetWindowTextLengthW(control) as usize + 1];
        let len = GetWindowTextW(control, &mut buf) as usize;
        String::from_utf16_lossy(&buf[..len])
    }
}

fn set_check(hwnd: HWND, id: i32, checked: bool) {
    if let Some(control) = item(hwnd, id) {
        unsafe { SendMessageW(control, BM_SETCHECK, Some(WPARAM(checked as usize)), None) };
    }
}

fn checked(hwnd: HWND, id: i32) -> bool {
    item(hwnd, id).is_some_and(|c| unsafe { SendMessageW(c, BM_GETCHECK, None, None).0 == 1 })
}

fn send(hwnd: HWND, id: i32, msg: u32, wparam: usize, lparam: isize) -> isize {
    item(hwnd, id).map_or(0, |c| unsafe { SendMessageW(c, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 })
}

fn enable(hwnd: HWND, id: i32, enabled: bool) {
    if let Some(control) = item(hwnd, id) {
        unsafe { _ = EnableWindow(control, enabled) };
    }
}

pub fn refresh() {
    let Some((hwnd, selected)) = with_settings(|s| {
        s.loading = true;
        (s.hwnd, s.selected)
    }) else {
        return;
    };
    let Some(config) = crate::config() else { return };
    send(hwnd, ID_LIST, LB_RESETCONTENT, 0, 0);
    for group in &config.groups {
        let name = HSTRING::from(group.name.as_str());
        send(hwnd, ID_LIST, LB_ADDSTRING, 0, name.as_ptr() as isize);
    }
    let selected = match selected {
        Some(i) if i < config.groups.len() => Some(i),
        _ if !config.groups.is_empty() => Some(0),
        _ => None,
    };
    send(hwnd, ID_LIST, LB_SETCURSEL, selected.map_or(usize::MAX, |i| i), 0);
    with_settings(|s| s.selected = selected);
    load_group(hwnd, selected);
    load_appearance(hwnd, &config.appearance);
    set_check(hwnd, ID_AGENT, crate::agent_enabled());
    set_check(hwnd, ID_AUTOSTART, crate::autostart::enabled());
    with_settings(|s| s.loading = false);
}

fn load_group(hwnd: HWND, selected: Option<usize>) {
    let group = selected.and_then(|i| crate::config()?.groups.get(i).cloned());
    for id in GROUP_FIELDS {
        enable(hwnd, id, group.is_some());
    }
    enable(hwnd, ID_COLLAPSED, group.is_some() && crate::agent_enabled());
    let previous = with_settings(|s| std::mem::replace(&mut s.loading, true));
    set_text(hwnd, ID_NAME, group.as_ref().map_or("", |g| g.name.as_str()));
    set_text(hwnd, ID_RULES, &group.as_ref().map_or(String::new(), |g| g.rules.join("\r\n")));
    set_check(hwnd, ID_COLLAPSED, group.as_ref().is_some_and(|g| g.collapsed));
    with_settings(|s| s.loading = previous.unwrap_or(false));
}

fn load_appearance(hwnd: HWND, appearance: &Appearance) {
    let theme = crate::themes::current(appearance).unwrap_or(crate::themes::THEMES.len());
    send(hwnd, ID_THEME, CB_SETCURSEL, theme, 0);
    send(hwnd, ID_HEADER_OPACITY, TBM_SETPOS, 1, appearance.header_opacity as isize);
    send(hwnd, ID_BODY_OPACITY, TBM_SETPOS, 1, appearance.body_opacity as isize);
    set_check(hwnd, ID_ANIMATE, appearance.animate);
    set_check(hwnd, ID_HOVER, appearance.hover_open);
    set_check(hwnd, ID_ROUNDED, appearance.rounded);
    let colors = [
        (ID_HEADER_SWATCH, &appearance.header_color),
        (ID_BODY_SWATCH, &appearance.body_color),
        (ID_TITLE_SWATCH, &appearance.title_color),
        (ID_BORDER_SWATCH, &appearance.border_color),
        (ID_LABEL_SWATCH, &appearance.label_color),
    ];
    with_settings(|s| {
        for (slot, (id, color)) in s.swatches.iter_mut().zip(colors) {
            unsafe {
                if !slot.1.is_invalid() {
                    _ = DeleteObject(slot.1.into());
                }
                *slot = (id, CreateSolidBrush(COLORREF(parse_color(color).unwrap_or(0))));
            }
        }
    });
    for id in [ID_HEADER_SWATCH, ID_BODY_SWATCH, ID_TITLE_SWATCH, ID_BORDER_SWATCH, ID_LABEL_SWATCH] {
        if let Some(control) = item(hwnd, id) {
            unsafe { _ = InvalidateRect(Some(control), None, true) };
        }
    }
}

fn change_appearance(hwnd: HWND, change: impl FnOnce(&mut Appearance)) {
    let Some(mut appearance) = crate::config().map(|c| c.appearance) else { return };
    change(&mut appearance);
    load_appearance(hwnd, &appearance);
    crate::set_appearance(appearance);
}

fn pick_color(hwnd: HWND, current: &str) -> Option<String> {
    let mut custom = with_settings(|s| s.custom)?;
    let mut choose = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: hwnd,
        rgbResult: COLORREF(parse_color(current).unwrap_or(0)),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        ..Default::default()
    };
    let picked = unsafe { ChooseColorW(&mut choose).as_bool() };
    with_settings(|s| s.custom = custom);
    picked.then(|| format_color(choose.rgbResult.0))
}

fn selected() -> Option<usize> {
    with_settings(|s| s.selected).flatten()
}

fn loading() -> bool {
    with_settings(|s| s.loading).unwrap_or(true)
}

fn flush_rules(hwnd: HWND) {
    unsafe { _ = KillTimer(Some(hwnd), TIMER_RULES) };
    let Some(index) = selected() else { return };
    let rules = text(hwnd, ID_RULES).lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
    crate::set_rules(index, rules);
}

fn command(hwnd: HWND, id: i32, code: u32) {
    match (id, code) {
        (ID_LIST, LBN_SELCHANGE) => {
            flush_rules(hwnd);
            let index = send(hwnd, ID_LIST, LB_GETCURSEL, 0, 0);
            let selected = (index >= 0).then_some(index as usize);
            with_settings(|s| s.selected = selected);
            load_group(hwnd, selected);
        }
        (ID_ADD, BN_CLICKED) => {
            flush_rules(hwnd);
            if let Some(index) = crate::add_group() {
                with_settings(|s| s.selected = Some(index));
            }
            refresh();
        }
        (ID_REMOVE, BN_CLICKED) => {
            let Some(index) = selected() else { return };
            let name = crate::config().and_then(|c| c.groups.get(index).map(|g| g.name.clone())).unwrap_or_default();
            let question = format!("Удалить группу «{name}»?\n\nФайлы на рабочем столе останутся на месте.");
            let answer = unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(question), w!("Corral"), MB_YESNO | MB_ICONQUESTION) };
            if answer == IDYES {
                unsafe { _ = KillTimer(Some(hwnd), TIMER_RULES) };
                crate::remove_group(index);
                refresh();
            }
        }
        (ID_NAME, EN_CHANGE) if !loading() => {
            let Some(index) = selected() else { return };
            let name = text(hwnd, ID_NAME);
            crate::rename_group(index, &name);
            with_settings(|s| s.loading = true);
            send(hwnd, ID_LIST, windows::Win32::UI::WindowsAndMessaging::LB_DELETESTRING, index, 0);
            let wide = HSTRING::from(name.as_str());
            send(hwnd, ID_LIST, windows::Win32::UI::WindowsAndMessaging::LB_INSERTSTRING, index, wide.as_ptr() as isize);
            send(hwnd, ID_LIST, LB_SETCURSEL, index, 0);
            with_settings(|s| s.loading = false);
        }
        (ID_RULES, EN_CHANGE) if !loading() => unsafe {
            SetTimer(Some(hwnd), TIMER_RULES, RULES_DELAY_MS, None);
        },
        (ID_COLLAPSED, BN_CLICKED) => {
            if let Some(index) = selected() {
                crate::set_collapsed(index, checked(hwnd, ID_COLLAPSED));
            }
        }
        (ID_HEADER_COLOR | ID_BODY_COLOR | ID_TITLE_COLOR | ID_BORDER_COLOR | ID_LABEL_COLOR, BN_CLICKED) => {
            let Some(appearance) = crate::config().map(|c| c.appearance) else { return };
            let current = match id {
                ID_HEADER_COLOR => appearance.header_color,
                ID_BODY_COLOR => appearance.body_color,
                ID_BORDER_COLOR => appearance.border_color,
                ID_LABEL_COLOR => appearance.label_color,
                _ => appearance.title_color,
            };
            if let Some(color) = pick_color(hwnd, &current) {
                change_appearance(hwnd, |a| match id {
                    ID_HEADER_COLOR => a.header_color = color,
                    ID_BODY_COLOR => a.body_color = color,
                    ID_BORDER_COLOR => a.border_color = color,
                    ID_LABEL_COLOR => a.label_color = color,
                    _ => a.title_color = color,
                });
            }
        }
        (ID_THEME, CBN_SELCHANGE) if !loading() => {
            let index = send(hwnd, ID_THEME, CB_GETCURSEL, 0, 0);
            if let Some(theme) = usize::try_from(index).ok().and_then(|i| crate::themes::THEMES.get(i)) {
                change_appearance(hwnd, |a| *a = theme.apply(a));
            }
        }
        (ID_ANIMATE, BN_CLICKED) => {
            let on = checked(hwnd, ID_ANIMATE);
            change_appearance(hwnd, |a| a.animate = on);
        }
        (ID_HOVER, BN_CLICKED) => {
            let on = checked(hwnd, ID_HOVER);
            change_appearance(hwnd, |a| a.hover_open = on);
        }
        (ID_ROUNDED, BN_CLICKED) => {
            let on = checked(hwnd, ID_ROUNDED);
            change_appearance(hwnd, |a| a.rounded = on);
        }
        (ID_AGENT, BN_CLICKED) => {
            crate::set_agent(checked(hwnd, ID_AGENT));
            refresh();
        }
        (ID_AUTOSTART, BN_CLICKED) => {
            crate::set_autostart(checked(hwnd, ID_AUTOSTART));
            if window().is_some() {
                set_check(hwnd, ID_AUTOSTART, crate::autostart::enabled());
            }
        }
        _ => {}
    }
}

unsafe extern "system" fn settings_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                let (id, code) = ((wparam.0 & 0xFFFF) as u16 as i16 as i32, ((wparam.0 >> 16) & 0xFFFF) as u32);
                command(hwnd, id, code);
                LRESULT(0)
            }
            WM_HSCROLL => {
                let header = item(hwnd, ID_HEADER_OPACITY);
                let body = item(hwnd, ID_BODY_OPACITY);
                let from = HWND(lparam.0 as _);
                let header_value = send(hwnd, ID_HEADER_OPACITY, TBM_GETPOS, 0, 0).clamp(0, 255) as u8;
                let body_value = send(hwnd, ID_BODY_OPACITY, TBM_GETPOS, 0, 0).clamp(0, 255) as u8;
                if Some(from) == header || Some(from) == body {
                    change_appearance(hwnd, |a| {
                        a.header_opacity = header_value;
                        a.body_opacity = body_value;
                    });
                }
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                let control = HWND(lparam.0 as _);
                let brush = with_settings(|s| {
                    s.swatches.iter().find(|(id, _)| item(hwnd, *id) == Some(control)).map(|(_, b)| *b)
                })
                .flatten();
                match brush {
                    Some(brush) => LRESULT(brush.0 as isize),
                    None => DefWindowProcW(hwnd, msg, wparam, lparam),
                }
            }
            WM_TIMER if wparam.0 == TIMER_RULES => {
                flush_rules(hwnd);
                LRESULT(0)
            }
            WM_CLOSE => {
                flush_rules(hwnd);
                _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                if let Some(settings) = SETTINGS.with(|cell| cell.borrow_mut().take()) {
                    _ = DeleteObject(settings.font.into());
                    for (_, brush) in settings.swatches {
                        if !brush.is_invalid() {
                            _ = DeleteObject(brush.into());
                        }
                    }
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
