use std::ffi::c_void;
use std::io;
use std::path::Path;
use std::ptr::{null, null_mut};

use super::display_local_path;

const CRED_TYPE_GENERIC: u32 = 1;
const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
const ERROR_NOT_FOUND: i32 = 1168;
const SW_SHOWNORMAL: i32 = 1;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

#[repr(C)]
struct Credential {
    flags: u32,
    kind: u32,
    target: *mut u16,
    comment: *mut u16,
    last_written: [u32; 2],
    blob_size: u32,
    blob: *mut u8,
    persist: u32,
    attributes_count: u32,
    attributes: *mut c_void,
    alias: *mut u16,
    username: *mut u16,
}

pub(crate) fn credential_exists(target: &str) -> io::Result<bool> {
    let target = wide(target);
    let mut credential = null_mut();
    // The target is NUL-terminated and the output pointer is writable.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
        let error = io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(ERROR_NOT_FOUND) => Ok(false),
            _ => Err(error),
        };
    }
    // Release the system buffer without reading or serializing its contents.
    unsafe { CredFree(credential) };
    Ok(true)
}

pub(crate) fn write_credential(target: &str, secret: &str) -> io::Result<()> {
    let mut target_wide = wide(target);
    let mut user_wide = wide("FlashMarkingDown");
    let mut blob: Vec<u8> = secret.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let blob_size = u32::try_from(blob.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "secret is too large"))?;
    let mut credential = Credential {
        flags: 0,
        kind: CRED_TYPE_GENERIC,
        target: target_wide.as_mut_ptr(),
        comment: null_mut(),
        last_written: [0; 2],
        blob_size,
        blob: blob.as_mut_ptr(),
        persist: CRED_PERSIST_LOCAL_MACHINE,
        attributes_count: 0,
        attributes: null_mut(),
        alias: null_mut(),
        username: user_wide.as_mut_ptr(),
    };
    let ok = unsafe { CredWriteW(&mut credential, 0) } != 0;
    let error = (!ok).then(io::Error::last_os_error);
    for byte in &mut blob {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    if let Some(error) = error {
        return Err(error);
    }
    Ok(())
}

pub(crate) fn read_credential(target: &str, allow_bearer: bool) -> io::Result<Option<String>> {
    let target = wide(target);
    let mut pointer = null_mut();
    // CredReadW owns the returned buffer until CredFree, including the credential blob.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let credential = unsafe { &*pointer.cast::<Credential>() };
    let token = if credential.blob_size == 0 {
        None
    } else {
        let blob =
            unsafe { std::slice::from_raw_parts(credential.blob, credential.blob_size as usize) };
        let normalize = |raw: &str| {
            let trimmed = raw.trim_end_matches('\0').trim();
            let key = if allow_bearer {
                super::bearer_token(trimmed).unwrap_or(trimmed)
            } else {
                trimmed
            };
            (!key.is_empty() && key.bytes().all(|b| b.is_ascii_graphic()))
                .then(|| trimmed.to_owned())
        };
        if let Ok(text) = std::str::from_utf8(blob)
            && let Some(token) = normalize(text)
        {
            Some(token)
        } else if blob.len().is_multiple_of(2) {
            let words: Vec<_> = blob
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16(&words)
                .ok()
                .and_then(|text| normalize(&text))
        } else {
            None
        }
    };
    unsafe { CredFree(pointer) };
    Ok(token)
}

pub(crate) fn open_file_default(path: &Path) -> Result<(), String> {
    let clean = display_local_path(path);
    let operation = wide("open");
    let target = wide(&clean);
    let handle = unsafe {
        ShellExecuteW(
            null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if (handle as usize) <= 32 {
        Err("无法使用系统默认程序打开该 Markdown 文件".into())
    } else {
        Ok(())
    }
}

pub(crate) fn reveal_file_in_explorer(path: &Path) -> Result<(), String> {
    let clean = display_local_path(path);
    std::process::Command::new("explorer.exe")
        .arg(format!("/select,{clean}"))
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[derive(Clone, Copy)]
pub(crate) struct ForegroundWindow {
    handle: isize,
    thread_id: u32,
}

impl ForegroundWindow {
    pub(crate) fn current() -> Option<Self> {
        Self::remember(unsafe { GetForegroundWindow() })
    }

    pub(crate) fn remember(handle: *mut c_void) -> Option<Self> {
        // The tray menu can temporarily activate its invisible owner window.
        if unsafe { IsWindowVisible(handle) } == 0 {
            return None;
        }
        let thread_id = unsafe { GetWindowThreadProcessId(handle, null_mut()) };
        (thread_id != 0).then_some(Self {
            handle: handle as isize,
            thread_id,
        })
    }

    pub(crate) fn restore(self, capture: isize) {
        let target = self.handle as *mut c_void;
        // A completed save must leave alone any app the user has since switched to.
        if unsafe { GetForegroundWindow() } as isize == capture
            && unsafe { GetWindowThreadProcessId(target, null_mut()) } == self.thread_id
            && unsafe { IsWindowVisible(target) } != 0
        {
            unsafe { SetForegroundWindow(target) };
        }
    }
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn CredReadW(
        target: *const u16,
        credential_type: u32,
        flags: u32,
        credential: *mut *mut c_void,
    ) -> i32;
    fn CredWriteW(credential: *mut Credential, flags: u32) -> i32;
    fn CredFree(credential: *mut c_void);
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn ShellExecuteW(
        hwnd: *mut c_void,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show_cmd: i32,
    ) -> *mut c_void;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(window: *mut c_void, process: *mut u32) -> u32;
    fn IsWindowVisible(window: *mut c_void) -> i32;
    fn SetForegroundWindow(window: *mut c_void) -> i32;
}
