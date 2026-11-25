use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::blocking::Client;
use serde_json::{Value, json};
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

pub(crate) fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("Bearer").then_some(token)
}

pub(crate) trait HttpTransport: Send + Sync {
    fn post_json(&self, url: &Url, token: Option<&str>, body: &[u8]) -> Result<Vec<u8>, String>;

    fn stream_chat(
        &self,
        url: &Url,
        model: &str,
        token: Option<&str>,
        messages: &[Value],
        on_chunk: &mut dyn FnMut(&str),
    ) -> Result<String, AppError> {
        let body = json!({"model": model, "stream": false, "messages": messages}).to_string();
        let bytes = self
            .post_json(url, token, body.as_bytes())
            .map_err(|msg| AppError::from_message("NETWORK_ERROR", "chat", true, msg))?;
        let parsed: Value = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::invalid("chat", "模型未返回有效 JSON 响应"))?;
        let text = parsed["choices"][0]["message"]["content"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AppError::invalid("chat", "模型未返回可用内容"))?;
        on_chunk(text);
        Ok(text.to_owned())
    }
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

    fn stream_chat(
        &self,
        url: &Url,
        model: &str,
        token: Option<&str>,
        messages: &[Value],
        on_chunk: &mut dyn FnMut(&str),
    ) -> Result<String, AppError> {
        let client = http_client(crate::model::loopback(url))?;
        let body = json!({"model": model, "stream": true, "messages": messages}).to_string();
        let mut request = client
            .post(url.clone())
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream, application/json")
            .body(body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().map_err(|error| {
            let message = if error.is_timeout() {
                "连接超时，请检查网络后重试"
            } else {
                "无法连接模型服务，请检查地址、网络或 TLS"
            };
            AppError::from_message("NETWORK_ERROR", "chat", true, message)
        })?;
        let status = response.status().as_u16();
        if !(200..=299).contains(&status) {
            return Err(http_status_error(
                "chat",
                status,
                read_error_detail(response),
            ));
        }
        let is_sse = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.to_ascii_lowercase().contains("text/event-stream"));
        if !is_sse {
            let bytes = read_bounded(response)?;
            let parsed: Value = serde_json::from_slice(&bytes)
                .map_err(|_| AppError::invalid("chat", "模型未返回有效响应"))?;
            let text = parsed["choices"][0]["message"]["content"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| AppError::invalid("chat", "模型未返回可用内容"))?;
            on_chunk(text);
            return Ok(text.to_owned());
        }
        let mut full = String::new();
        let mut reader = BufReader::new(response);
        let mut line = String::new();
        const MAX_STREAM_BYTES: usize = 512 * 1024;
        loop {
            line.clear();
            let n = reader.read_line(&mut line).map_err(|_| {
                AppError::from_message("NETWORK_ERROR", "chat", true, "流式响应读取中断")
            })?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            let Some(payload) = trimmed.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                break;
            }
            if let Ok(event) = serde_json::from_str::<Value>(payload) {
                if let Some(delta) = event["choices"][0]["delta"]["content"].as_str()
                    && !delta.is_empty()
                {
                    if full.len() + delta.len() > MAX_STREAM_BYTES {
                        return Err(AppError::invalid("chat", "模型响应超过本地处理上限"));
                    }
                    full.push_str(delta);
                    on_chunk(delta);
                } else if full.is_empty()
                    && let Some(content) = event["choices"][0]["message"]["content"].as_str()
                    && !content.is_empty()
                {
                    full.push_str(content);
                    on_chunk(content);
                }
            }
        }
        if full.trim().is_empty() {
            return Err(AppError::invalid("chat", "模型未返回可用内容"));
        }
        Ok(full)
    }
}

fn http_client(direct: bool) -> Result<&'static Client, AppError> {
    static DIRECT: OnceLock<Result<Client, reqwest::Error>> = OnceLock::new();
    static PROXIED: OnceLock<Result<Client, reqwest::Error>> = OnceLock::new();
    let client = if direct { &DIRECT } else { &PROXIED };
    client
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
        })
}

fn read_error_detail(response: reqwest::blocking::Response) -> Option<String> {
    const ERROR_LIMIT: u64 = 16 * 1024;
    let mut bytes = Vec::new();
    response.take(ERROR_LIMIT).read_to_end(&mut bytes).ok()?;
    let raw = String::from_utf8_lossy(&bytes);
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        for candidate in [
            value["error"]["message"].as_str(),
            value["error"].as_str(),
            value["message"].as_str(),
            value["detail"].as_str(),
        ] {
            if let Some(msg) = candidate.map(str::trim).filter(|m| !m.is_empty()) {
                return Some(msg.to_owned());
            }
        }
    }
    (!trimmed.starts_with('<')).then(|| trimmed.to_owned())
}

fn http_status_error(stage: &'static str, status: u16, detail: Option<String>) -> AppError {
    let summary = detail.as_deref().map(|text| {
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let count = flat.chars().count();
        let clipped: String = flat.chars().take(160).collect();
        if count > 160 {
            format!("{clipped}…")
        } else {
            clipped
        }
    });
    match status {
        401 | 403 => AppError::new(
            "AUTH_FAILED",
            stage,
            false,
            summary
                .map(|s| format!("认证失败或权限不足（HTTP {status}）：{s}"))
                .unwrap_or_else(|| format!("认证失败或权限不足（HTTP {status}），请检查 API Key")),
            detail,
        ),
        300..=399 => AppError::invalid(stage, "端点返回重定向，请直接配置目标地址"),
        404 => AppError::new(
            "SERVICE_ERROR",
            stage,
            false,
            summary
                .map(|s| format!("服务返回 HTTP 404（请核对接口地址或模型名称）：{s}"))
                .unwrap_or_else(|| "服务返回 HTTP 404，请核对接口地址或模型名称".to_owned()),
            detail,
        ),
        _ => AppError::new(
            "SERVICE_ERROR",
            stage,
            status >= 500 || status == 429,
            summary
                .map(|s| format!("服务返回 HTTP {status}：{s}"))
                .unwrap_or_else(|| format!("服务返回 HTTP {status}，请检查配置后重试")),
            detail,
        ),
    }
}

fn read_bounded(response: reqwest::blocking::Response) -> Result<Vec<u8>, AppError> {
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

pub(crate) fn request_json(
    url: &Url,
    auth: Auth<'_>,
    body: Option<&[u8]>,
) -> Result<Vec<u8>, AppError> {
    request_json_inner(url, auth, body, None, true)
}

// Memory requests have a shorter deadline and never carry upstream error text into job state.
pub(crate) fn request_json_with_timeout(
    url: &Url,
    auth: Auth<'_>,
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<Vec<u8>, AppError> {
    request_json_inner(url, auth, body, Some(timeout), false)
}

// `body.is_some()` sends a JSON POST; `None` sends a GET.
fn request_json_inner(
    url: &Url,
    auth: Auth<'_>,
    body: Option<&[u8]>,
    timeout: Option<Duration>,
    include_error_detail: bool,
) -> Result<Vec<u8>, AppError> {
    let client = http_client(crate::model::loopback(url))?;
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
    if let Some(timeout) = timeout {
        request = request.timeout(timeout);
    }
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
    if !(200..=299).contains(&status) {
        if status == 404 && !include_error_detail {
            return Err(AppError::from_message(
                "NOT_FOUND",
                "connection",
                false,
                "服务未找到指定接口或内容（HTTP 404）",
            ));
        }
        let detail = if include_error_detail {
            read_error_detail(response)
        } else {
            None
        };
        let mut error = http_status_error("connection", status, detail);
        error.retryable |= !include_error_detail && status == 409;
        return Err(error);
    }
    read_bounded(response)
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
