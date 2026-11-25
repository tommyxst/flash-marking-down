use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::{MemoryProvider, OpenVikingProvider, Url, Value, json};

pub(crate) struct MockRequest {
    pub method: String,
    pub target: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Value>,
}

pub(crate) fn mock_http(
    expected_requests: usize,
    mut respond: impl FnMut(MockRequest) -> (u16, Value) + Send + 'static,
) -> (Url, JoinHandle<()>) {
    mock_http_response(expected_requests, move |request| {
        let (status, value) = respond(request);
        (status, "application/json", value.to_string())
    })
}

pub(crate) fn mock_http_response(
    expected_requests: usize,
    mut respond: impl FnMut(MockRequest) -> (u16, &'static str, String) + Send + 'static,
) -> (Url, JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        for _ in 0..expected_requests {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("mock HTTP listener did not receive a request: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut first_line = String::new();
            reader.read_line(&mut first_line).unwrap();
            let mut request_line = first_line.split_whitespace();
            let method = request_line.next().unwrap().to_owned();
            let target = request_line.next().unwrap().to_owned();
            let mut headers = HashMap::new();
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                let (name, value) = line.split_once(':').unwrap();
                headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
            }
            let body = headers.get("content-length").and_then(|length| {
                let length = length.parse().unwrap();
                if length == 0 {
                    return None;
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                Some(serde_json::from_slice(&bytes).unwrap())
            });
            let (status, content_type, result) = respond(MockRequest {
                method,
                target,
                headers,
                body,
            });
            write!(
                socket,
                "HTTP/1.1 {status} Mock\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{result}",
                result.len()
            )
            .unwrap();
        }
    });
    (url, server)
}

#[test]
fn checks_health_with_bearer_and_accepts_anonymous_dev_mode() {
    let (mut url, server) = mock_http(1, |request| {
        assert_eq!(request.method, "GET");
        assert_eq!(request.target, "/proxy/health");
        assert_eq!(request.headers["authorization"], "Bearer synthetic");
        assert!(!request.headers.contains_key("x-api-key"));
        (
            200,
            json!({
                "status": "ok", "healthy": true, "version": "0.5.2",
                "auth_mode": "api_key", "account_id": "demo", "user_id": "reader", "role": "user"
            }),
        )
    });
    url.set_path("/proxy/");
    let provider = OpenVikingProvider::new(url, "fixture".into(), Some("bEaReR synthetic".into()));
    assert_eq!(provider.provider_id(), "openviking");
    let health = provider.health_check().unwrap();
    assert!(health.healthy);
    assert_eq!(health.version.as_deref(), Some("0.5.2"));
    assert_eq!(health.identity.as_deref(), Some("账户 demo，用户 reader"));
    server.join().unwrap();

    let (url, server) = mock_http(1, |request| {
        assert_eq!(request.target, "/health");
        assert!(!request.headers.contains_key("authorization"));
        assert!(!request.headers.contains_key("x-api-key"));
        (
            200,
            json!({"status": "ok", "healthy": true, "version": "0.5.2", "auth_mode": "dev"}),
        )
    });
    let provider = OpenVikingProvider::new(url, "fixture".into(), None);
    let health = provider.health_check().unwrap();
    assert!(health.healthy);
    assert!(health.identity.is_none());
    server.join().unwrap();
}

#[test]
fn writes_full_resource_with_valid_tags_and_uses_uri_fallback() {
    const URI: &str = "viking://resources/flash-marking-down/fixture/daily/2026-11-25.md";
    let (url, server) = mock_http(2, |request| {
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/api/v1/content/write");
        assert_eq!(request.headers["x-api-key"], "synthetic");
        let body = request.body.unwrap();
        let mut expected =
            json!({"uri": URI, "content": "# 当日原文", "mode": "replace", "wait": false});
        let result = if body.get("tags").is_some() {
            expected["tags"] = json!(["app=flash-marking-down", "workspace=fixture"]);
            json!({"uri": "viking://resources/flash-marking-down/fixture/normalized.md"})
        } else {
            json!({})
        };
        assert_eq!(body, expected);
        (200, json!({"status": "ok", "result": result}))
    });
    let provider = OpenVikingProvider::new(url, "fixture".into(), Some("synthetic".into()));
    let path = "daily/2026-11-25.md";
    let invalid = ["bare", "extra=value=value", "space=two values", "missing="].map(str::to_owned);
    let mut valid = invalid.to_vec();
    valid.extend(["app=flash-marking-down".into(), "Workspace=Fixture".into()]);
    assert!(
        provider
            .upsert_daily_note(path, "# 当日原文", &valid)
            .unwrap()
            .ends_with("/normalized.md")
    );
    assert_eq!(
        provider
            .upsert_daily_note(path, "# 当日原文", &invalid)
            .unwrap(),
        URI
    );
    server.join().unwrap();
}

#[test]
fn commits_a_user_message_without_wait_parameter() {
    let mut step = 0;
    let (url, server) = mock_http(3, move |request| {
        let (path, body, result) = match step {
            0 => (
                "/api/v1/sessions",
                json!({}),
                json!({"session_id": "new-session"}),
            ),
            1 => (
                "/api/v1/sessions/new-session/messages",
                json!({"role": "user", "content": "# 笔记\n整理要点"}),
                json!({"message_count": 1}),
            ),
            _ => (
                "/api/v1/sessions/new-session/commit",
                json!({}),
                json!({"status": "accepted", "task_id": "processing-task"}),
            ),
        };
        step += 1;
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, path);
        assert_eq!(request.body, Some(body));
        (200, json!({"status": "ok", "result": result}))
    });
    let provider = OpenVikingProvider::new(url, "fixture".into(), None);
    let task = provider
        .commit_note_memory("笔记", "# 笔记\n整理要点", &["app=fixture".into()])
        .unwrap();
    assert_eq!(task.as_deref(), Some("processing-task"));
    server.join().unwrap();
}

#[test]
fn searches_and_ranks_both_kinds_with_derived_titles_and_snippets() {
    let (url, server) = mock_http(1, |request| {
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/api/v1/search/search");
        assert_eq!(
            request.body,
            Some(json!({
                "query": "回环检索", "limit": 3, "mode": "list",
                "context_type": ["memory", "resource"], "read_content": true,
                "since": "2026-11-01", "until": "2026-11-25"
            }))
        );
        (
            200,
            json!({"status": "ok", "result": {
                "memories": [
                    {
                        "uri": "viking://user/demo/memories/first.md", "score": 0.8,
                        "content": "---\r\ndirectory: viking://user/demo/memories\r\n---\r\n# 首行标题\n无关开头\n前文\n回环检索应同时返回两种类型",
                        "abstract": "备选描述"
                    },
                    {"uri": "viking://user/demo/memories/lowest.md", "score": 0.1},
                    {"uri": "https://example.invalid/content", "score": 1.0}
                ],
                "resources": [
                    {
                        "uri": "viking://resources/example/highest.md", "score": 0.9,
                        "abstract": "从摘要提取标题\n仍保留摘要内容"
                    },
                    {"uri": "viking://resources/example/name-from-uri.md", "score": 0.6}
                ]
            }}),
        )
    });
    let provider = OpenVikingProvider::new(url, "fixture".into(), None);
    let hits = provider
        .search("回环检索", 3, Some("2026-11-01"), Some("2026-11-25"))
        .unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].kind, "ov_resource");
    assert_eq!(hits[0].title, "从摘要提取标题");
    assert!(hits[0].snippet.contains("仍保留摘要内容"));
    assert_eq!(hits[1].kind, "ov_memory");
    assert_eq!(hits[1].title, "首行标题");
    assert!(hits[1].snippet.contains("回环检索"));
    assert!(!hits[1].snippet.contains("无关开头"));
    assert!(!hits[1].snippet.contains("directory:"));
    assert_eq!(hits[2].title, "name-from-uri");
    server.join().unwrap();
}

#[test]
fn encodes_reference_uris_and_reads_document_or_overview() {
    const FILE: &str = "viking://resources/测试 空格=值&更多.md";
    const DIR: &str = "viking://resources/测试目录";
    let (url, server) = mock_http(3, |request| {
        assert_eq!(request.method, "GET");
        let url = Url::parse(&format!("http://localhost{}", request.target)).unwrap();
        let query = url.query_pairs().collect::<Vec<_>>();
        assert_eq!(query.len(), 1);
        assert_eq!(query[0].0, "uri");
        match (url.path(), query[0].1.as_ref()) {
            ("/api/v1/content/read", FILE) => {
                (200, json!({"status": "ok", "result": "# 完整 Markdown"}))
            }
            ("/api/v1/content/read", DIR) => (404, json!({"status": "error"})),
            ("/api/v1/content/overview", DIR) => {
                (200, json!({"status": "ok", "result": "# 目录概览"}))
            }
            (path, _) => panic!("unexpected mock request: {path}"),
        }
    });
    let provider = OpenVikingProvider::new(url, "fixture".into(), None);
    assert_eq!(
        provider
            .read_content("https://example.invalid/file")
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
    assert_eq!(provider.read_content(FILE).unwrap(), "# 完整 Markdown");
    assert_eq!(provider.read_content(DIR).unwrap(), "# 目录概览");
    server.join().unwrap();
}

#[test]
fn excludes_remote_error_text_from_diagnostics() {
    const REMOTE_DETAIL: &str = "synthetic private response body";
    let (url, server) = mock_http(4, |request| match request.target.as_str() {
        "/health" => (
            403,
            json!({"status": "error", "error": {
                "code": "PERMISSION_DENIED", "message": REMOTE_DETAIL
            }}),
        ),
        "/api/v1/search/search" => (
            200,
            json!({"status": "error", "error": {
                "code": "UNAVAILABLE", "message": REMOTE_DETAIL
            }}),
        ),
        "/api/v1/content/write" | "/api/v1/sessions" => (
            if request.target.ends_with("/write") {
                409
            } else {
                200
            },
            json!({"status": "error", "error": {
                "code": "CONFLICT", "message": REMOTE_DETAIL
            }}),
        ),
        path => panic!("unexpected mock request: {path}"),
    });
    let provider = OpenVikingProvider::new(url, "fixture".into(), Some("synthetic".into()));
    let health = provider.health_check().unwrap_err();
    assert_eq!(health.code, "AUTH_FAILED");
    assert!(health.safe_message.contains("HTTP 403"));
    let search = provider.search("回环", 1, None, None).unwrap_err();
    assert_eq!(search.code, "SERVICE_ERROR");
    assert!(search.retryable);
    let http_conflict = provider
        .upsert_daily_note("daily/fixture.md", "回环原文", &[])
        .unwrap_err();
    let envelope_conflict = provider
        .commit_note_memory("回环", "回环笔记", &[])
        .unwrap_err();
    assert!(http_conflict.safe_message.contains("HTTP 409"));
    for error in [&http_conflict, &envelope_conflict] {
        assert_eq!(error.code, "SERVICE_ERROR");
        assert!(error.retryable);
    }
    for error in [health, search, http_conflict, envelope_conflict] {
        assert!(error.technical_detail.is_none());
        assert!(!error.safe_message.contains(REMOTE_DETAIL));
    }
    server.join().unwrap();
}
