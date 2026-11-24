use serde_json::{Value, json};
use tauri::Url;

use crate::platform::{HttpTransport, default_http_transport};

const JSON_RETRY_LIMIT: usize = 1;
const JSON_RETRY_NOTICE: &str = "上次回复不是合法的单个 JSON 对象。请仅返回一个符合要求的纯 JSON 对象，不要附加任何说明文字或代码块标记。";

pub(crate) fn endpoint(value: &str) -> Result<Url, String> {
    let url = Url::parse(value.trim()).map_err(|_| "模型端点必须是完整 URL")?;
    let local_or_lan = is_local_or_lan(&url);
    if !(url.scheme() == "https" || url.scheme() == "http" && local_or_lan)
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path().is_empty()
    {
        return Err(
            "请填写完整的模型 HTTP(S) URL；公网端点须使用 HTTPS，地址中不能包含凭据或片段".into(),
        );
    }
    Ok(url)
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
    let user_content = input.to_string();
    for attempt in 0..=JSON_RETRY_LIMIT {
        let system = if attempt == 0 {
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
        match extract_json_object(text) {
            Ok(value) => return Ok(value),
            Err(error) if attempt == JSON_RETRY_LIMIT => return Err(error),
            Err(_) => continue,
        }
    }
    Err("模型输出不符合要求的 JSON 格式".into())
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
        let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = server.local_addr().unwrap();
        let expected = endpoint(&format!(
            "http://{address}/v1/chat/completions?api-version=1"
        ))
        .unwrap();
        let peer = std::thread::spawn(move || {
            let (mut connection, _) = server.accept().unwrap();
            connection
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut connection);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(
                line.trim_end(),
                "POST /v1/chat/completions?api-version=1 HTTP/1.1"
            );
            let mut content_length = None;
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                assert!(!line.to_ascii_lowercase().starts_with("authorization:"));
                let (key, value) = line.split_once(':').unwrap();
                if key.eq_ignore_ascii_case("content-length") {
                    content_length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
            let mut bytes = vec![0; content_length.unwrap()];
            reader.read_exact(&mut bytes).unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(request["model"], "fixture");
            assert_eq!(request["stream"], false);
            connection.write_all(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        assert!(
            generate(
                &expected,
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
}
