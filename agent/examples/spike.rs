use std::time::{Duration, Instant};

use corral_agent::protocol::{self, CMD_DETACH, CMD_SET_HIDDEN};
use windows::Win32::Foundation::{FreeLibrary, HWND, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowThreadProcessId, HOOKPROC, HWND_MESSAGE, PostMessageW, SendMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_GETMESSAGE, WM_COPYDATA, WM_NULL,
};
use windows::core::{HSTRING, PCSTR, w};

fn main() {
    let mut args = std::env::args().skip(1);
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).expect("usage: spike <seconds> <name>...");
    let names: Vec<String> = args.collect();

    unsafe {
        let exe = std::env::current_exe().unwrap();
        let built = exe.parent().unwrap().parent().unwrap().join("corral_agent.dll");
        let copy = std::env::temp_dir().join(format!("corral_agent_{}.dll", std::process::id()));
        std::fs::copy(&built, &copy).expect("copy agent dll");

        let module = LoadLibraryW(&HSTRING::from(copy.as_os_str())).expect("load agent");
        let proc = GetProcAddress(module, PCSTR(protocol::HOOK_PROC.as_ptr())).expect("hook proc");
        let hook_proc: HOOKPROC = Some(std::mem::transmute(proc));

        let progman = FindWindowW(w!("Progman"), None).expect("Progman");
        let def_view = FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), None).expect("DefView");
        let list_view = FindWindowExW(Some(def_view), None, w!("SysListView32"), None).expect("ListView");
        let thread = GetWindowThreadProcessId(list_view, None);

        let hook = SetWindowsHookExW(WH_GETMESSAGE, hook_proc, Some(module.into()), thread).expect("hook");
        _ = PostMessageW(Some(list_view), WM_NULL, WPARAM(0), LPARAM(0));

        let class = HSTRING::from(protocol::WINDOW_CLASS);
        let started = Instant::now();
        let agent = loop {
            if let Ok(agent) = FindWindowExW(Some(HWND_MESSAGE), None, &class, None) {
                break Some(agent);
            }
            if started.elapsed() > Duration::from_secs(3) {
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };

        match agent {
            Some(agent) => {
                println!("agent attached in {:?}", started.elapsed());
                send(agent, CMD_SET_HIDDEN, &protocol::encode_names(&names));
                println!("hidden: {names:?}, waiting {seconds}s");
                std::thread::sleep(Duration::from_secs(seconds));
                send(agent, CMD_DETACH, &[]);
                println!("detached");
            }
            None => println!("agent did not attach"),
        }

        _ = UnhookWindowsHookEx(hook);
        _ = FreeLibrary(module);
    }
}

unsafe fn send(agent: HWND, command: usize, data: &[u16]) {
    let payload = COPYDATASTRUCT {
        dwData: command,
        cbData: (data.len() * 2) as u32,
        lpData: if data.is_empty() { std::ptr::null_mut() } else { data.as_ptr() as *mut _ },
    };
    unsafe { SendMessageW(agent, WM_COPYDATA, None, Some(LPARAM(&payload as *const _ as isize))) };
}
