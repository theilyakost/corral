use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use corral_agent::protocol::{self, CMD_DETACH, CMD_SET_HIDDEN, CMD_SET_OWNER};
use windows::Win32::Foundation::{FreeLibrary, HMODULE, HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowThreadProcessId, HHOOK, HOOKPROC, HWND_MESSAGE, IsWindow, PostMessageW, SendMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_GETMESSAGE, WM_COPYDATA, WM_NULL,
};
use windows::core::{Error, HSTRING, PCSTR, Result};

pub struct Agent {
    hook: HHOOK,
    module: HMODULE,
    window: HWND,
    hidden: Vec<String>,
    pub started: Instant,
}

impl Agent {
    pub fn inject(list_view: HWND) -> Result<Agent> {
        let source = std::env::current_exe().map_err(|_| Error::empty())?.with_file_name("corral_agent.dll");
        let copy = staging_copy(&source).ok_or_else(Error::empty)?;
        unsafe {
            let module = LoadLibraryW(&HSTRING::from(copy.as_os_str()))?;
            let Some(proc) = GetProcAddress(module, PCSTR(protocol::HOOK_PROC.as_ptr())) else {
                _ = FreeLibrary(module);
                return Err(Error::empty());
            };
            let hook_proc: HOOKPROC = Some(std::mem::transmute(proc));
            let thread = GetWindowThreadProcessId(list_view, None);
            let hook = match SetWindowsHookExW(WH_GETMESSAGE, hook_proc, Some(module.into()), thread) {
                Ok(hook) => hook,
                Err(e) => {
                    _ = FreeLibrary(module);
                    return Err(e);
                }
            };
            _ = PostMessageW(Some(list_view), WM_NULL, WPARAM(0), LPARAM(0));

            let class = HSTRING::from(protocol::WINDOW_CLASS);
            let started = Instant::now();
            let window = loop {
                if let Ok(window) = FindWindowExW(Some(HWND_MESSAGE), None, &class, None) {
                    break Some(window);
                }
                if started.elapsed() > Duration::from_secs(2) {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            let Some(window) = window else {
                _ = UnhookWindowsHookEx(hook);
                _ = FreeLibrary(module);
                return Err(Error::empty());
            };

            let agent = Agent { hook, module, window, hidden: Vec::new(), started };
            let pid = std::process::id();
            agent.send(CMD_SET_OWNER, &[pid as u16, (pid >> 16) as u16]);
            Ok(agent)
        }
    }

    pub fn alive(&self) -> bool {
        unsafe { IsWindow(Some(self.window)).as_bool() }
    }

    pub fn set_hidden(&mut self, mut names: Vec<String>) {
        names.sort();
        if names != self.hidden {
            self.send(CMD_SET_HIDDEN, &protocol::encode_names(&names));
            self.hidden = names;
        }
    }

    fn send(&self, command: usize, data: &[u16]) {
        let payload = COPYDATASTRUCT {
            dwData: command,
            cbData: (data.len() * 2) as u32,
            lpData: if data.is_empty() { std::ptr::null_mut() } else { data.as_ptr() as *mut _ },
        };
        unsafe { SendMessageW(self.window, WM_COPYDATA, None, Some(LPARAM(&payload as *const _ as isize))) };
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        if self.alive() {
            self.send(CMD_DETACH, &[]);
        }
        unsafe {
            _ = UnhookWindowsHookEx(self.hook);
            _ = FreeLibrary(self.module);
        }
    }
}

fn staging_copy(source: &PathBuf) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("corral");
    std::fs::create_dir_all(&dir).ok()?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            _ = std::fs::remove_file(entry.path());
        }
    }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let copy = dir.join(format!("agent-{}-{stamp}.dll", std::process::id()));
    std::fs::copy(source, &copy).ok()?;
    Some(copy)
}

pub fn disabled_flag() -> PathBuf {
    crate::config::Config::path().with_file_name("agent.disabled")
}
