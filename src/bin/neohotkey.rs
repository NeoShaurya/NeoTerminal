#![windows_subsystem = "windows"]

use std::process::Command;

#[repr(C)]
#[derive(Clone, Copy)]
struct KBDLLHOOKSTRUCT {
    vk_code: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[repr(C)]
struct POINT {
    x: i32,
    y: i32,
}

#[repr(C)]
struct MSG {
    hwnd: *mut std::ffi::c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: POINT,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(lpModuleName: *const u16) -> *mut std::ffi::c_void;
    fn CreateMutexW(
        lpMutexAttributes: *mut std::ffi::c_void,
        bInitialOwner: i32,
        lpName: *const u16,
    ) -> *mut std::ffi::c_void;
    fn GetLastError() -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn SetWindowsHookExW(
        idHook: i32,
        lpfn: Option<unsafe extern "system" fn(i32, usize, isize) -> isize>,
        hmod: *mut std::ffi::c_void,
        dwThreadId: u32,
    ) -> *mut std::ffi::c_void;
    fn CallNextHookEx(hhk: *mut std::ffi::c_void, nCode: i32, wParam: usize, lParam: isize) -> isize;
    fn GetMessageW(lpMsg: *mut MSG, hWnd: *mut std::ffi::c_void, wMsgFilterMin: u32, wMsgFilterMax: u32) -> i32;
    fn TranslateMessage(lpMsg: *const MSG) -> i32;
    fn DispatchMessageW(lpMsg: *const MSG) -> isize;
    fn GetAsyncKeyState(vKey: i32) -> i16;
}

const WH_KEYBOARD_LL: i32 = 13;
const WM_KEYDOWN: usize = 0x0100;
const WM_SYSKEYDOWN: usize = 0x0104;

const VK_LWIN: i32 = 0x5B;
const VK_RWIN: i32 = 0x5C;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_Q: u32 = 0x51;
const VK_T: u32 = 0x54;

static mut LAST_LAUNCH: u32 = 0;

unsafe extern "system" fn hook_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code >= 0 && (wparam == WM_KEYDOWN || wparam == WM_SYSKEYDOWN) {
        let kbd = *(lparam as *const KBDLLHOOKSTRUCT);
        let win_down = GetAsyncKeyState(VK_LWIN) < 0 || GetAsyncKeyState(VK_RWIN) < 0;
        let ctrl_down = GetAsyncKeyState(VK_CONTROL) < 0;
        let alt_down = GetAsyncKeyState(VK_MENU) < 0;

        let is_win_q = win_down && kbd.vk_code == VK_Q;
        let is_ctrl_alt_t = ctrl_down && alt_down && kbd.vk_code == VK_T;

        if is_win_q || is_ctrl_alt_t {
            if kbd.time.wrapping_sub(LAST_LAUNCH) > 600 {
                LAST_LAUNCH = kbd.time;
                let exe_path = if std::path::Path::new("C:\\Users\\shaur\\NeoTerminal.exe").exists() {
                    "C:\\Users\\shaur\\NeoTerminal.exe"
                } else {
                    "C:\\Users\\shaur\\Documents\\antigravity\\bold-galileo\\NeoTerminal\\target\\release\\NeoTerminal.exe"
                };
                let _ = Command::new(exe_path).spawn();
            }
            return 1;
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

fn main() {
    unsafe {
        // Enforce single-instance mutex to prevent duplicate daemons
        let mutex_name: Vec<u16> = "Global\\NeoTerminalHotkeyDaemonMutex\0"
            .encode_utf16()
            .collect();
        let mutex = CreateMutexW(std::ptr::null_mut(), 0, mutex_name.as_ptr());
        if mutex.is_null() || GetLastError() == 183 {
            return;
        }

        let hmod = GetModuleHandleW(std::ptr::null());
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), hmod, 0);
        if hook.is_null() {
            return;
        }

        let mut msg: MSG = std::mem::zeroed();
        loop {
            let res = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if res <= 0 {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
