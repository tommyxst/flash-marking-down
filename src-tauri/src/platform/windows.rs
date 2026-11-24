use std::ffi::c_void;
use std::io;
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use tauri::Url;

use super::{HttpTransport, display_local_path};
use crate::model::loopback;

const NO_PROXY: u32 = 1;
const AUTOMATIC_PROXY: u32 = 4;
const SECURE: u32 = 0x0080_0000;
const DISABLE_FEATURE: u32 = 63;
const DISABLED_FEATURES: u32 = 1 | 2 | 4;
const STATUS_NUMBER: u32 = 19 | 0x2000_0000;
const CONNECTION_TIMEOUT_MS: i32 = 10_000;
const RESPONSE_TIMEOUT_MS: i32 = 60_000;
const RESPONSE_LIMIT: usize = 512 * 1024;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(90);

const CRED_TYPE_GENERIC: u32 = 1;
const CRED_PERSIST_LOCAL_MACHINE: u32 = 2;
const ERROR_NOT_FOUND: i32 = 1168;
const SW_SHOWNORMAL: i32 = 1;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

pub(crate) struct WinHttpTransport;

type Raw = *mut c_void;

struct WinHttpHandle(Raw);

impl WinHttpHandle {
    fn new(raw: Raw) -> Result<Self, String> {
        if raw.is_null() {
            Err(network_error())
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for WinHttpHandle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn network_error() -> String {
    let code = io::Error::last_os_error().raw_os_error();
    if code == Some(12002) {
        "模型请求超时，可以重试".into()
    } else {
        format!("模型连接失败，请检查端点、网络或 TLS（WinHTTP {code:?}）")
    }
}

fn check(result: i32) -> Result<(), String> {
    if result == 0 {
        Err(network_error())
    } else {
        Ok(())
    }
}

impl HttpTransport for WinHttpTransport {
    fn post_json(&self, url: &Url, token: Option<&str>, body: &[u8]) -> Result<Vec<u8>, String> {
        let started = Instant::now();
        let agent = wide("Flash Marking Down");
        let access = if loopback(url) {
            NO_PROXY
        } else {
            AUTOMATIC_PROXY
        };
        let host = wide(
            url.host_str()
                .ok_or("模型地址缺少主机")?
                .trim_matches(['[', ']']),
        );
        let port = url.port_or_known_default().ok_or("模型地址缺少端口")?;
        let verb = wide("POST");
        let request_target = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        let path = wide(&request_target);
        let flags = if url.scheme() == "https" { SECURE } else { 0 };
        let length = u32::try_from(body.len()).map_err(|_| "模型请求过大")?;
        let mut headers =
            String::from("Content-Type: application/json\r\nAccept: application/json\r\n");
        if let Some(token) = token {
            headers.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        let headers = wide(&headers);
        // The handles are synchronous and their input buffers live through each call.
        unsafe {
            let http = WinHttpHandle::new(WinHttpOpen(agent.as_ptr(), access, null(), null(), 0))?;
            check(WinHttpSetTimeouts(
                http.0,
                CONNECTION_TIMEOUT_MS,
                CONNECTION_TIMEOUT_MS,
                CONNECTION_TIMEOUT_MS,
                RESPONSE_TIMEOUT_MS,
            ))?;
            let connection = WinHttpHandle::new(WinHttpConnect(http.0, host.as_ptr(), port, 0))?;
            let request = WinHttpHandle::new(WinHttpOpenRequest(
                connection.0,
                verb.as_ptr(),
                path.as_ptr(),
                null(),
                null(),
                null(),
                flags,
            ))?;
            let features = DISABLED_FEATURES;
            check(WinHttpSetOption(
                request.0,
                DISABLE_FEATURE,
                (&raw const features).cast(),
                size_of::<u32>() as u32,
            ))?;
            check(WinHttpSendRequest(
                request.0,
                headers.as_ptr(),
                u32::MAX,
                body.as_ptr().cast(),
                length,
                length,
                0,
            ))?;
            check(WinHttpReceiveResponse(request.0, null_mut()))?;
            let mut status: u32 = 0;
            let mut size = size_of::<u32>() as u32;
            check(WinHttpQueryHeaders(
                request.0,
                STATUS_NUMBER,
                null(),
                (&raw mut status).cast(),
                &mut size,
                null_mut(),
            ))?;
            match status {
                200..=299 => {}
                401 | 403 => return Err(format!("模型端点拒绝认证或权限不足（HTTP {status}）")),
                300..=399 => return Err("模型端点返回重定向，请核对并直接配置目标地址".into()),
                _ => return Err(format!("模型端点返回 HTTP {status}，请检查配置后重试")),
            }
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 8192];
            loop {
                if started.elapsed() > TOTAL_TIMEOUT {
                    return Err("模型请求超时，可以重试".into());
                }
                let mut read = 0;
                check(WinHttpReadData(
                    request.0,
                    chunk.as_mut_ptr().cast(),
                    chunk.len() as u32,
                    &mut read,
                ))?;
                if read == 0 {
                    break;
                }
                if bytes.len() + read as usize > RESPONSE_LIMIT {
                    return Err("模型输出超过本地处理上限".into());
                }
                bytes.extend_from_slice(&chunk[..read as usize]);
            }
            Ok(bytes)
        }
    }
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

pub(crate) fn read_credential(target: &str) -> io::Result<Option<String>> {
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
            (!trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_graphic()))
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

#[link(name = "winhttp")]
unsafe extern "system" {
    fn WinHttpOpen(
        agent: *const u16,
        access: u32,
        proxy: *const u16,
        bypass: *const u16,
        flags: u32,
    ) -> Raw;
    fn WinHttpSetTimeouts(handle: Raw, resolve: i32, connect: i32, send: i32, receive: i32) -> i32;
    fn WinHttpConnect(handle: Raw, host: *const u16, port: u16, reserved: u32) -> Raw;
    fn WinHttpOpenRequest(
        connection: Raw,
        verb: *const u16,
        path: *const u16,
        version: *const u16,
        referrer: *const u16,
        accepts: *const *const u16,
        flags: u32,
    ) -> Raw;
    fn WinHttpSetOption(handle: Raw, option: u32, buffer: *const c_void, length: u32) -> i32;
    fn WinHttpSendRequest(
        request: Raw,
        headers: *const u16,
        header_length: u32,
        body: *const c_void,
        bytes: u32,
        total: u32,
        context: usize,
    ) -> i32;
    fn WinHttpReceiveResponse(handle: Raw, reserved: Raw) -> i32;
    fn WinHttpQueryHeaders(
        handle: Raw,
        level: u32,
        name: *const u16,
        buffer: Raw,
        length: *mut u32,
        index: *mut u32,
    ) -> i32;
    fn WinHttpReadData(handle: Raw, buffer: Raw, size: u32, read: *mut u32) -> i32;
    fn WinHttpCloseHandle(handle: Raw) -> i32;
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
