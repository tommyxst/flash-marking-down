use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::Url;

use crate::dto::AppError;
use crate::memory::{MemoryProvider, OpenVikingProvider};
use crate::platform::{Auth, HttpTransport, default_http_transport, request_json};

#[derive(Serialize, Deserialize)]
pub(crate) struct ModelSettings {
    pub endpoint: String,
    pub model: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemoryConfig {
    pub base_url: String,
    pub namespace: String,
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub sync_resources: bool,
    #[serde(default = "default_true")]
    pub commit_memories: bool,
}

fn default_true() -> bool {
    true
}

const JSON_RETRY_LIMIT: usize = 1;
const JSON_RETRY_NOTICE: &str = "上次回复不是合法的单个 JSON 对象。请仅返回一个符合要求的纯 JSON 对象，不要附加任何说明文字或代码块标记。";

pub(crate) fn endpoint(value: &str) -> Result<Url, String> {
    let url = Url::parse(value.trim()).map_err(|_| "服务地址必须是完整 URL")?;
    let local_or_lan = is_local_or_lan(&url);
    if !(url.scheme() == "https" || url.scheme() == "http" && local_or_lan)
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query_pairs().any(|(key, _)| {
            let key = key.to_ascii_lowercase().replace(['-', '_'], "");
            key.ends_with("key")
                || key.ends_with("token")
                || key.ends_with("secret")
                || key.ends_with("signature")
                || matches!(
                    key.as_str(),
                    "sig" | "password" | "credential" | "authorization"
                )
        })
        || url.path().is_empty()
    {
        return Err(
            "请填写完整的 HTTP(S) URL。公网端点须使用 HTTPS，地址中不能包含凭据或片段".into(),
        );
    }
    Ok(url)
}

pub(crate) fn chat_endpoint(value: &str) -> Result<Url, String> {
    let mut url = endpoint(value)?;
    let trimmed = url.path().trim_end_matches('/').to_owned();
    if trimmed.is_empty() {
        url.set_path("/v1/chat/completions");
    } else if trimmed.ends_with("/v1") || trimmed.ends_with("/v4") {
        url.set_path(&format!("{trimmed}/chat/completions"));
    }
    Ok(url)
}

pub(crate) fn models_endpoint(value: &str) -> Result<Url, String> {
    let mut url = chat_endpoint(value)?;
    let trimmed = url.path().trim_end_matches('/').to_owned();
    let next = if let Some(prefix) = trimmed.strip_suffix("/chat/completions") {
        format!("{prefix}/models")
    } else {
        format!("{trimmed}/models")
    };
    url.set_path(&next);
    Ok(url)
}

pub(crate) fn list_models(url: &Url, token: Option<&str>) -> Result<Vec<String>, AppError> {
    let response = request_json(url, token.map_or(Auth::None, Auth::Bearer), None)?;
    let result: Value = serde_json::from_slice(&response)
        .map_err(|_| AppError::invalid("model_config", "端点未返回有效的 /models JSON 列表"))?;
    let mut models = Vec::new();
    if let Some(items) = result["data"].as_array() {
        for item in items {
            if let Some(id) = item["id"].as_str().map(str::trim).filter(|s| !s.is_empty())
                && !models.iter().any(|existing: &String| existing == id)
            {
                models.push(id.to_owned());
            }
        }
    }
    if models.is_empty() {
        return Err(AppError::invalid(
            "model_config",
            "端点未返回可用的模型列表（/models）",
        ));
    }
    Ok(models)
}

impl ModelSettings {
    pub(crate) fn new(url: &str, model: &str, enabled: bool) -> Result<Self, String> {
        let model = model.trim();
        if model.is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
            return Err("请填写有效的模型名称".into());
        }
        Ok(Self {
            endpoint: chat_endpoint(url)?.to_string(),
            model: model.into(),
            enabled,
        })
    }
}

impl MemoryConfig {
    pub(crate) fn new(
        base_url: &str,
        namespace: &str,
        enabled: bool,
        sync_resources: bool,
        commit_memories: bool,
    ) -> Result<Self, String> {
        let url = endpoint(base_url)?;
        if url.query().is_some() {
            return Err("OpenViking 服务地址不能包含查询参数".into());
        }
        let name = namespace.trim();
        if name.is_empty()
            || matches!(name, "." | "..")
            || !name
                .chars()
                .all(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        {
            return Err("命名空间可使用中英文、数字、点、连字符和下划线".into());
        }
        Ok(Self {
            base_url: url.as_str().trim_end_matches('/').to_owned(),
            namespace: name.into(),
            enabled,
            sync_resources,
            commit_memories,
        })
    }
}

pub(crate) fn probe_model(url: &Url, model: &str, token: Option<&str>) -> Result<String, AppError> {
    let request = json!({"model": model, "stream": false, "messages": [{"role":"user","content":"Reply with OK."}]}).to_string();
    let response = request_json(
        url,
        token.map_or(Auth::None, Auth::Bearer),
        Some(request.as_bytes()),
    )?;
    let result: Value = serde_json::from_slice(&response)
        .map_err(|_| AppError::invalid("connection", "端点未返回有效的 Chat Completions JSON"))?;
    if result["choices"][0]["message"]["content"]
        .as_str()
        .is_none_or(|v| v.trim().is_empty())
    {
        return Err(AppError::invalid(
            "connection",
            "服务已响应，但没有返回模型回复，请核对 Chat Completions 地址与模型名",
        ));
    }
    let actual = result["model"].as_str().unwrap_or(model);
    Ok(format!("连接成功，模型 {actual} 已响应。"))
}

pub(crate) fn probe_memory(base_url: &Url, token: Option<&str>) -> Result<String, AppError> {
    let provider =
        OpenVikingProvider::new(base_url.clone(), String::new(), token.map(str::to_owned));
    let health = provider.health_check()?;
    if !health.healthy {
        return Err(AppError::invalid(
            "connection",
            "OpenViking 未报告就绪，请检查服务状态",
        ));
    }
    Ok(match (health.version, health.identity) {
        (Some(version), Some(identity)) => {
            format!("连接成功（OpenViking {version}），{identity}。")
        }
        (None, Some(identity)) => format!("连接成功，{identity}。"),
        (Some(version), None) => format!("连接成功（OpenViking {version}，免密开发模式）。"),
        (None, None) => "连接成功，服务处于免密开发模式。".into(),
    })
}

pub(crate) fn loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let bare = host.trim_matches(['[', ']']);
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

pub(crate) fn is_local_or_lan(url: &Url) -> bool {
    if loopback(url) {
        return true;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let bare = host.trim_matches(['[', ']']);
    if bare.to_ascii_lowercase().ends_with(".local") {
        return true;
    }
    match bare.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => {
            (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

pub(crate) fn generate(
    url: &Url,
    model: &str,
    token: Option<&str>,
    instruction: &str,
    input: Value,
) -> Result<Value, String> {
    generate_with(
        default_http_transport(),
        url,
        model,
        token,
        instruction,
        input,
    )
}

pub(crate) fn generate_with(
    transport: &dyn HttpTransport,
    url: &Url,
    model: &str,
    token: Option<&str>,
    instruction: &str,
    input: Value,
) -> Result<Value, String> {
    let mut retries = JSON_RETRY_LIMIT;
    generate_validated(
        transport,
        url,
        model,
        token,
        instruction,
        input,
        &mut retries,
        Ok,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_validated<T>(
    transport: &dyn HttpTransport,
    url: &Url,
    model: &str,
    token: Option<&str>,
    instruction: &str,
    input: Value,
    retries: &mut usize,
    validate: impl Fn(Value) -> Result<T, String>,
) -> Result<T, String> {
    let user_content = input.to_string();
    let mut retry = false;
    loop {
        let system = if !retry {
            instruction.to_owned()
        } else {
            format!("{instruction}\n\n{JSON_RETRY_NOTICE}")
        };
        let body = json!({"model":model, "stream":false, "messages":[
            {"role":"system", "content":system},
            {"role":"user", "content":user_content}
        ]})
        .to_string();
        let bytes = transport.post_json(url, token, body.as_bytes())?;
        let result: Value =
            serde_json::from_slice(&bytes).map_err(|_| "模型端点未返回有效的 JSON")?;
        let message = &result["choices"][0];
        if matches!(
            message["finish_reason"].as_str(),
            Some("length" | "content_filter" | "tool_calls")
        ) {
            return Err("模型未完成这次生成，请缩小资料范围后重试".into());
        }
        let text = message["message"]["content"]
            .as_str()
            .ok_or("模型未返回可用内容")?;
        match extract_json_object(text).and_then(&validate) {
            Ok(value) => return Ok(value),
            Err(error) if *retries == 0 => return Err(error),
            Err(_) => {
                *retries -= 1;
                retry = true;
            }
        }
    }
}

fn extract_json_object(raw: &str) -> Result<Value, String> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed)
        && value.is_object()
    {
        return Ok(value);
    }
    if let Some(fenced) = strip_code_fence(trimmed)
        && let Ok(value) = serde_json::from_str::<Value>(fenced)
        && value.is_object()
    {
        return Ok(value);
    }
    if let Some(value) = find_embedded_json_object(trimmed) {
        return Ok(value);
    }
    Err("模型输出不符合要求的 JSON 格式".into())
}

fn strip_code_fence(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("```")?;
    let (header, body) = rest.split_once('\n')?;
    if !header
        .trim()
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return None;
    }
    let body = body.trim_end();
    Some(body.strip_suffix("```")?.trim())
}

fn find_embedded_json_object(text: &str) -> Option<Value> {
    let mut offset = 0;
    while let Some(rel_start) = text[offset..].find('{') {
        let start = offset + rel_start;
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for (rel_idx, ch) in text[start..].char_indices() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '"' => in_string = true,
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let end = start + rel_idx;
                        if let Ok(value) = serde_json::from_str::<Value>(&text[start..=end])
                            && value.is_object()
                        {
                            return Some(value);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
        offset = start + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::time::Duration;

    struct StubTransport {
        responses: Mutex<Vec<Result<Vec<u8>, String>>>,
        seen_tokens: Mutex<Vec<Option<String>>>,
    }

    fn reply_once(
        expected_request: &'static str,
        auth: &'static str,
        status: &'static str,
        body: &'static str,
    ) -> (Url, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = endpoint(&format!("http://{}/proxy", listener.local_addr().unwrap())).unwrap();
        let peer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            assert_eq!(first.trim_end(), expected_request);
            let mut headers = Vec::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                let lower = line.trim().to_lowercase();
                if let Some(value) = lower.strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                headers.push(lower);
            }
            if auth.is_empty() {
                assert!(
                    !headers
                        .iter()
                        .any(|h| h.starts_with("authorization:") || h.starts_with("x-api-key:"))
                );
            } else {
                assert!(headers.iter().any(|h| h == auth));
            }
            let mut data = vec![0; length];
            reader.read_exact(&mut data).unwrap();
            if first.starts_with("POST") {
                let request: Value = serde_json::from_slice(&data).unwrap();
                if auth.is_empty() || auth.starts_with("authorization:") {
                    assert_eq!(request["model"], "fixture");
                    assert_eq!(request["stream"], false);
                    if !auth.is_empty() {
                        assert_eq!(request["messages"][0]["content"], "Reply with OK.");
                    }
                }
            }
            write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (url, peer)
    }

    #[test]
    fn probes_confirm_model_and_authenticated_memory_identity() {
        let (url, peer) = reply_once(
            "POST /proxy HTTP/1.1",
            "authorization: bearer synthetic",
            "200 OK",
            r#"{"model":"fixture-v1","choices":[{"message":{"content":"OK"}}]}"#,
        );
        assert!(
            probe_model(&url, "fixture", Some("synthetic"))
                .unwrap()
                .contains("fixture-v1")
        );
        peer.join().unwrap();
        let (url, peer) = reply_once(
            "GET /proxy/health HTTP/1.1",
            "x-api-key: synthetic",
            "200 OK",
            r#"{"status":"ok","account_id":"demo","user_id":"reader"}"#,
        );
        assert!(
            probe_memory(&url, Some("synthetic"))
                .unwrap()
                .contains("账户 demo，用户 reader")
        );
        peer.join().unwrap();
    }

    #[test]
    fn probes_distinguish_authentication_and_network_errors() {
        for (status, body) in [
            ("401 Unauthorized", "{}"),
            ("403 Forbidden", "{}"),
            ("200 OK", r#"{"status":"ok","auth_mode":"api_key"}"#),
            (
                "200 OK",
                r#"{"status":"ok","auth_mode":"api_key","account_id":"default","user_id":"default","role":"root"}"#,
            ),
        ] {
            let (url, peer) = reply_once(
                "GET /proxy/health HTTP/1.1",
                "x-api-key: synthetic",
                status,
                body,
            );
            let error = probe_memory(&url, Some("synthetic")).unwrap_err();
            assert_eq!(error.code, "AUTH_FAILED");
            if body.contains("\"root\"") {
                assert!(error.safe_message.contains("root"));
            }
            peer.join().unwrap();
        }
        for token in [None, Some("synthetic")] {
            let auth = if token.is_some() {
                "x-api-key: synthetic"
            } else {
                ""
            };
            let (url, peer) = reply_once(
                "GET /proxy/health HTTP/1.1",
                auth,
                "200 OK",
                r#"{"status":"ok","version":"0.5.2","auth_mode":"dev"}"#,
            );
            let message = probe_memory(&url, token).unwrap();
            assert!(message.contains("0.5.2") && message.contains("免密开发模式"));
            peer.join().unwrap();
        }
        let (url, peer) = reply_once(
            "POST /proxy HTTP/1.1",
            "x-api-key: synthetic",
            "200 OK",
            r#"{"ok":true}"#,
        );
        assert_eq!(
            request_json(
                &url,
                Auth::ApiKey("synthetic"),
                Some(br#"{"uri":"viking://a"}"#)
            )
            .unwrap(),
            br#"{"ok":true}"#
        );
        peer.join().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = endpoint(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        drop(listener);
        assert_eq!(
            probe_model(&url, "fixture", None).unwrap_err().code,
            "NETWORK_ERROR"
        );
    }

    impl HttpTransport for StubTransport {
        fn post_json(
            &self,
            _url: &Url,
            token: Option<&str>,
            _body: &[u8],
        ) -> Result<Vec<u8>, String> {
            self.seen_tokens
                .lock()
                .unwrap()
                .push(token.map(str::to_owned));
            self.responses.lock().unwrap().remove(0)
        }
    }

    #[test]
    fn generate_with_mock_transport_validates_finish_reason_and_retries() {
        let stub = StubTransport {
            responses: Mutex::new(vec![
                Ok(
                    json!({"choices":[{"finish_reason":"stop","message":{"content":"not json"}}]})
                        .to_string()
                        .into_bytes(),
                ),
                Ok(
                    json!({"choices":[{"finish_reason":"stop","message":{"content":"{\"ok\":true}"}}]})
                        .to_string()
                        .into_bytes(),
                ),
                Ok(
                    json!({"choices":[{"finish_reason":"length","message":{"content":"{\"ok\":true}"}}]})
                        .to_string()
                        .into_bytes(),
                ),
            ]),
            seen_tokens: Mutex::new(Vec::new()),
        };
        let url = endpoint("https://api.example.com/v1/chat/completions").unwrap();
        let value =
            generate_with(&stub, &url, "demo", Some("sk-demo"), "sys", json!({"a": 1})).unwrap();
        assert_eq!(value["ok"], json!(true));
        assert_eq!(stub.seen_tokens.lock().unwrap().len(), 2);
        let truncated = generate_with(&stub, &url, "demo", None, "sys", json!({"a": 1}));
        assert!(truncated.unwrap_err().contains("缩小资料范围"));
    }

    #[cfg(windows)]
    #[test]
    fn request_uses_the_approved_endpoint_and_does_not_follow_redirects() {
        assert!(endpoint("http://example.com/v1/chat/completions").is_err());
        for key in [
            "api%2Dkey",
            "accessToken",
            "subscription-key",
            "X-Amz-Signature",
        ] {
            assert!(
                endpoint(&format!(
                    "https://example.org/v1/chat/completions?{key}=fixture"
                ))
                .is_err()
            );
        }
        assert!(endpoint("https://user:pass@example.com/v1/chat/completions").is_err());
        assert!(endpoint("http://localhost:11434/v1/chat/completions").is_ok());
        assert!(endpoint("http://192.168.1.20:8000/v1/chat/completions").is_ok());
        assert!(
            endpoint(
                "https://example.com/openai/deployments/d/chat/completions?api-version=2024-02-01"
            )
            .is_ok()
        );
        assert!(is_local_or_lan(
            &endpoint("http://192.168.1.20:8000/v1/chat/completions").unwrap()
        ));
        assert!(!is_local_or_lan(
            &endpoint("https://api.example.com/v1/chat/completions").unwrap()
        ));
        let (mut url, peer) = reply_once(
            "POST /v1/chat/completions?api-version=1 HTTP/1.1",
            "",
            "307 Temporary Redirect\r\nLocation: /elsewhere",
            "",
        );
        url.set_path("/v1/chat/completions");
        url.set_query(Some("api-version=1"));
        assert!(
            generate(
                &url,
                "fixture",
                None,
                "fixture",
                json!({"body":"synthetic"})
            )
            .unwrap_err()
            .contains("重定向")
        );
        peer.join().unwrap();
    }

    #[test]
    fn extracts_json_from_code_fences_and_surrounding_prose() {
        assert_eq!(
            extract_json_object(r#"{"items":[]}"#).unwrap()["items"],
            json!([])
        );
        assert_eq!(
            extract_json_object("```JSON\r\n{\"items\":[{\"text\":\"含 \\\"}\\\" 转义\"}]}\r\n```")
                .unwrap()["items"][0]["text"],
            json!("含 \"}\" 转义")
        );
        assert_eq!(
            extract_json_object("```\n{\"ok\":true}\n```").unwrap()["ok"],
            json!(true)
        );
        assert_eq!(
            extract_json_object("说明 {不是JSON}，结果如下：\n```json\n{\"ok\":1}\n```\n结束")
                .unwrap()["ok"],
            json!(1)
        );
        assert!(extract_json_object("[1,2,3]").is_err());
        assert!(extract_json_object("纯文本回复").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn retries_once_when_model_returns_invalid_json_format() {
        let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = server.local_addr().unwrap();
        let url = endpoint(&format!("http://{address}/v1/chat/completions")).unwrap();
        let peer = std::thread::spawn(move || {
            for round in 0..2 {
                let (mut connection, _) = server.accept().unwrap();
                connection
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&mut connection);
                let mut line = String::new();
                let mut content_length = 0usize;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some((key, value)) = line.split_once(':')
                        && key.eq_ignore_ascii_case("content-length")
                    {
                        content_length = value.trim().parse().unwrap();
                    }
                }
                let mut bytes = vec![0; content_length];
                reader.read_exact(&mut bytes).unwrap();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                if round == 1 {
                    assert!(
                        request["messages"][0]["content"]
                            .as_str()
                            .unwrap()
                            .contains("纯 JSON 对象")
                    );
                }
                let content = if round == 0 {
                    "抱歉，这次没有按 JSON 返回"
                } else {
                    "```\n{\"items\":[]}\n```"
                };
                let payload =
                    json!({"choices":[{"finish_reason":"stop","message":{"content":content}}]})
                        .to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                );
                connection.write_all(response.as_bytes()).unwrap();
            }
        });
        let value = generate(&url, "fixture", None, "instruction", json!({"a":1})).unwrap();
        assert_eq!(value["items"], json!([]));
        peer.join().unwrap();
    }

    #[test]
    fn normalizes_base_urls_lists_models_and_surfaces_404_detail() {
        assert_eq!(
            chat_endpoint("http://127.0.0.1:3425").unwrap().as_str(),
            "http://127.0.0.1:3425/v1/chat/completions"
        );
        assert_eq!(
            chat_endpoint("http://127.0.0.1:3425/v1").unwrap().as_str(),
            "http://127.0.0.1:3425/v1/chat/completions"
        );
        assert_eq!(
            chat_endpoint("http://127.0.0.1:3425/v1/").unwrap().as_str(),
            "http://127.0.0.1:3425/v1/chat/completions"
        );
        assert_eq!(
            chat_endpoint("https://open.bigmodel.cn/api/paas/v4")
                .unwrap()
                .as_str(),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        assert_eq!(
            models_endpoint("http://127.0.0.1:3425/v1")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:3425/v1/models"
        );
        assert_eq!(
            models_endpoint("http://127.0.0.1:3425/v1/chat/completions")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:3425/v1/models"
        );

        let (url, peer) = reply_once(
            "GET /proxy/models HTTP/1.1",
            "",
            "200 OK",
            r#"{"data":[{"id":"auto/gemini-3.8-flash"},{"id":"group/deepseek-v4.1-flash"},{"id":"auto/gemini-3.8-flash"}]}"#,
        );
        let mut models_url = url;
        models_url.set_path("/proxy/models");
        assert_eq!(
            list_models(&models_url, None).unwrap(),
            vec!["auto/gemini-3.8-flash", "group/deepseek-v4.1-flash"]
        );
        peer.join().unwrap();

        let (url, peer) = reply_once(
            "POST /proxy HTTP/1.1",
            "",
            "404 Not Found",
            r#"{"error":{"message":"magpie knows no model \"gpt-4.1-mini\""}}"#,
        );
        let error = probe_model(&url, "fixture", None).unwrap_err();
        assert_eq!(error.code, "SERVICE_ERROR");
        assert!(error.safe_message.contains("HTTP 404"));
        assert!(error.safe_message.contains("magpie knows no model"));
        peer.join().unwrap();
    }
}
