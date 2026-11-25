use std::ffi::c_void;
use std::io;
use std::ptr::null_mut;

pub(crate) const WM_HOTKEY: u32 = 0x0312;
const WM_USER: u32 = 0x0400;
const WM_APP: u32 = 0x8000;
pub(crate) const SETTINGS_MESSAGE: u32 = WM_APP;
pub(crate) const STOP_MESSAGE: u32 = WM_APP + 1;
const PM_NOREMOVE: u32 = 0;
pub(crate) const MOD_ALT: u32 = 0x0001;
pub(crate) const MOD_CONTROL: u32 = 0x0002;
pub(crate) const MOD_WIN: u32 = 0x0008;
const MOD_NOREPEAT: u32 = 0x4000;
const ERROR_HOTKEY_ALREADY_REGISTERED: i32 = 1409;

#[repr(C)]
struct Message {
    window: *mut c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point: [i32; 2],
    private: u32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterHotKey(window: *mut c_void, id: i32, modifiers: u32, key: u32) -> i32;
    fn UnregisterHotKey(window: *mut c_void, id: i32) -> i32;
    fn PeekMessageW(
        message: *mut Message,
        window: *mut c_void,
        min: u32,
        max: u32,
        mode: u32,
    ) -> i32;
    fn GetMessageW(message: *mut Message, window: *mut c_void, min: u32, max: u32) -> i32;
    fn PostThreadMessageW(thread: u32, message: u32, wparam: usize, lparam: isize) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

#[derive(Clone, Copy)]
pub(crate) struct Binding {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) registration_id: i32,
    pub(crate) modifiers: u32,
    pub(crate) key: u32,
}

pub(crate) struct Registration(pub(crate) &'static Binding);

impl Registration {
    pub(crate) fn register(binding: &'static Binding) -> Result<Self, String> {
        if unsafe {
            RegisterHotKey(
                null_mut(),
                binding.registration_id,
                binding.modifiers | MOD_NOREPEAT,
                binding.key,
            )
        } != 0
        {
            return Ok(Self(binding));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_HOTKEY_ALREADY_REGISTERED) {
            Err(format!("{} 已被 Windows 或其他应用占用", binding.label))
        } else {
            Err(format!("无法注册快捷键 {}（{error}）", binding.label))
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if unsafe { UnregisterHotKey(null_mut(), self.0.registration_id) } == 0 {
            let error = io::Error::last_os_error();
            eprintln!("无法注销快捷键 {}（{error}）", self.0.label);
        }
    }
}

pub(crate) fn post(thread: u32, message: u32) -> io::Result<()> {
    if unsafe { PostThreadMessageW(thread, message, 0, 0) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn init_message_queue() -> u32 {
    let mut message = unsafe { std::mem::zeroed() };
    unsafe {
        PeekMessageW(&mut message, null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
        GetCurrentThreadId()
    }
}

pub(crate) fn next_message() -> io::Result<Option<(u32, usize)>> {
    let mut message: Message = unsafe { std::mem::zeroed() };
    match unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } {
        0 => Ok(None),
        -1 => Err(io::Error::last_os_error()),
        _ => Ok(Some((message.message, message.wparam))),
    }
}
