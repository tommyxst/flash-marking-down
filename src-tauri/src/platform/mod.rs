use std::io;
use std::path::Path;

use tauri::Url;

#[cfg(windows)]
mod windows;

pub(crate) trait HttpTransport: Send + Sync {
    fn post_json(&self, url: &Url, token: Option<&str>, body: &[u8]) -> Result<Vec<u8>, String>;
}

pub(crate) fn default_http_transport() -> &'static dyn HttpTransport {
    #[cfg(windows)]
    {
        &windows::WinHttpTransport
    }
    #[cfg(not(windows))]
    {
        &UnsupportedHttpTransport
    }
}

#[cfg(not(windows))]
struct UnsupportedHttpTransport;

#[cfg(not(windows))]
impl HttpTransport for UnsupportedHttpTransport {
    fn post_json(&self, _url: &Url, _token: Option<&str>, _body: &[u8]) -> Result<Vec<u8>, String> {
        Err("当前平台暂未提供 HTTP 传输实现".into())
    }
}

#[cfg(windows)]
pub(crate) use windows::{
    ForegroundWindow, credential_exists, open_file_default, read_credential,
    reveal_file_in_explorer, write_credential,
};

pub(crate) fn display_local_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }
    #[cfg(not(windows))]
    {
        text.into_owned()
    }
}

pub(crate) fn is_sharing_violation(error: &io::Error) -> bool {
    #[cfg(windows)]
    {
        matches!(error.raw_os_error(), Some(32 | 33))
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        false
    }
}

pub(crate) fn zero_string(value: &mut String) {
    // SAFETY: writing ASCII NUL bytes preserves valid UTF-8 before clearing the buffer.
    unsafe {
        for byte in value.as_bytes_mut() {
            std::ptr::write_volatile(byte, 0);
        }
    }
    value.clear();
}
