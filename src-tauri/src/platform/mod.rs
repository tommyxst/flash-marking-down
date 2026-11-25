use std::io::{self, Read};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::blocking::Client;
use tauri::Url;

use crate::dto::AppError;

#[cfg(windows)]
pub(crate) mod hotkey;
#[cfg(windows)]
mod windows;

#[derive(Clone, Copy)]
pub(crate) enum Auth<'a> {
    None,
    Bearer(&'a str),
    ApiKey(&'a str),
}

pub(crate) trait HttpTransport: Send + Sync {
    fn post_json(&self, url: &Url, token: Option<&str>, body: &[u8]) -> Result<Vec<u8>, String>;
}

pub(crate) fn default_http_transport() -> &'static dyn HttpTransport {
    &ReqwestTransport
}

struct ReqwestTransport;

impl HttpTransport for ReqwestTransport {
    fn post_json(&self, url: &Url, token: Option<&str>, body: &[u8]) -> Result<Vec<u8>, String> {
        request_json(url, token.map_or(Auth::None, Auth::Bearer), Some(body))
            .map_err(|error| error.safe_message)
    }
}

// `body.is_some()` sends a JSON POST; `None` sends a GET.
pub(crate) fn request_json(
    url: &Url,
    auth: Auth<'_>,
    body: Option<&[u8]>,
) -> Result<Vec<u8>, AppError> {
    static DIRECT: OnceLock<Result<Client, reqwest::Error>> = OnceLock::new();
    static PROXIED: OnceLock<Result<Client, reqwest::Error>> = OnceLock::new();
    let direct = crate::model::loopback(url);
    let client = if direct { &DIRECT } else { &PROXIED };
    let client = client
        .get_or_init(|| {
            let mut builder = Client::builder()
                .user_agent("Flash Marking Down")
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(90))
                .redirect(reqwest::redirect::Policy::none());
            if direct {
                builder = builder.no_proxy();
            }
            builder.build()
        })
        .as_ref()
        .map_err(|_| {
            AppError::from_message(
                "NETWORK_ERROR",
                "connection",
                true,
                "无法初始化网络连接，请检查系统代理或 TLS 配置",
            )
        })?;
    let mut request = if let Some(body) = body {
        client
            .post(url.clone())
            .header("Content-Type", "application/json")
            .body(body.to_vec())
    } else {
        client.get(url.clone())
    };
    request = match auth {
        Auth::None => request,
        Auth::Bearer(token) => request.bearer_auth(token),
        Auth::ApiKey(token) => request.header("X-API-Key", token),
    };
    let response = request
        .header("Accept", "application/json")
        .send()
        .map_err(|error| {
            let message = if error.is_timeout() {
                "连接超时，请检查网络后重试"
            } else {
                "无法连接服务，请检查地址、网络或 TLS"
            };
            AppError::from_message("NETWORK_ERROR", "connection", true, message)
        })?;
    let status = response.status().as_u16();
    match status {
        200..=299 => {}
        401 | 403 => {
            return Err(AppError::from_message(
                "AUTH_FAILED",
                "connection",
                false,
                format!("认证失败或权限不足（HTTP {status}），请检查 API Key"),
            ));
        }
        300..=399 => {
            return Err(AppError::invalid(
                "connection",
                "端点返回重定向，请直接配置目标地址",
            ));
        }
        _ => {
            return Err(AppError::from_message(
                "SERVICE_ERROR",
                "connection",
                true,
                format!("服务返回 HTTP {status}，请检查配置后重试"),
            ));
        }
    }
    const RESPONSE_LIMIT: u64 = 512 * 1024;
    let mut bytes = Vec::new();
    response
        .take(RESPONSE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            AppError::from_message(
                "NETWORK_ERROR",
                "connection",
                true,
                "读取响应失败，请检查网络后重试",
            )
        })?;
    if bytes.len() as u64 > RESPONSE_LIMIT {
        return Err(AppError::invalid("connection", "服务响应超过本地处理上限"));
    }
    Ok(bytes)
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
        if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{share}");
        }
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }
    #[cfg(not(windows))]
    {
        text.into_owned()
    }
}

#[cfg(all(windows, test))]
#[test]
fn displays_shell_paths_for_network_shares_and_drives() {
    assert_eq!(
        display_local_path(Path::new(r"\\?\UNC\server\share\note.md")),
        r"\\server\share\note.md"
    );
    assert_eq!(display_local_path(Path::new(r"\\?\C:\notes")), r"C:\notes");
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
