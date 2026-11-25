use std::time::Duration;

use serde_json::{Value, json};
use tauri::Url;

use crate::dto::AppError;
use crate::platform::{Auth, bearer_token, request_json_with_timeout};
use crate::storage::m2_answer::{extract_query_terms, focus_snippet};
use crate::storage::note_title;

#[cfg(test)]
pub(crate) mod tests;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const SEARCH_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug)]
pub(crate) struct MemoryHealth {
    pub healthy: bool,
    pub version: Option<String>,
    pub identity: Option<String>,
}

#[derive(Debug)]
pub(crate) struct MemoryHit {
    pub uri: String,
    pub kind: String,
    pub title: String,
    pub snippet: String,
    pub score: Option<f64>,
    pub updated_at: Option<String>,
}

pub(crate) trait MemoryProvider: Send + Sync {
    fn provider_id(&self) -> &'static str;
    fn health_check(&self) -> Result<MemoryHealth, AppError>;
    fn upsert_daily_note(
        &self,
        relative_uri: &str,
        markdown: &str,
        tags: &[String],
    ) -> Result<String, AppError>;
    fn commit_note_memory(
        &self,
        title: &str,
        markdown: &str,
        tags: &[String],
    ) -> Result<Option<String>, AppError>;
    fn search(
        &self,
        query: &str,
        limit: usize,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<MemoryHit>, AppError>;
    fn read_content(&self, uri: &str) -> Result<String, AppError>;
}

pub(crate) struct OpenVikingProvider {
    base_url: Url,
    namespace: String,
    token: Option<String>,
}

impl OpenVikingProvider {
    pub(crate) fn new(base_url: Url, namespace: String, token: Option<String>) -> Self {
        Self {
            base_url,
            namespace,
            token,
        }
    }

    pub(crate) fn resource_uri(&self, relative_uri: &str) -> String {
        format!(
            "viking://resources/flash-marking-down/{}/{relative_uri}",
            self.namespace
        )
    }

    fn endpoint(&self, path: &[&str]) -> Result<Url, AppError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| AppError::invalid("connection", "OpenViking 服务地址无效"))?
            .pop_if_empty()
            .extend(path);
        Ok(url)
    }

    fn request(
        &self,
        url: &Url,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, AppError> {
        let auth = match self.token.as_deref() {
            None => Auth::None,
            Some(token) => bearer_token(token).map_or(Auth::ApiKey(token), Auth::Bearer),
        };
        let body = body.map(|value| value.to_string());
        let bytes =
            request_json_with_timeout(url, auth, body.as_deref().map(str::as_bytes), timeout)?;
        serde_json::from_slice(&bytes).map_err(|_| invalid_response())
    }

    fn post(&self, path: &[&str], body: Value, timeout: Duration) -> Result<Value, AppError> {
        unpack(self.request(&self.endpoint(path)?, Some(body), timeout)?)
    }

    fn read(&self, route: &str, uri: &str) -> Result<String, AppError> {
        let mut url = self.endpoint(&["api", "v1", "content", route])?;
        url.query_pairs_mut().append_pair("uri", uri);
        unpack(self.request(&url, None, REQUEST_TIMEOUT)?)?
            .as_str()
            .map(str::to_owned)
            .ok_or_else(invalid_response)
    }
}

impl MemoryProvider for OpenVikingProvider {
    fn provider_id(&self) -> &'static str {
        "openviking"
    }

    fn health_check(&self) -> Result<MemoryHealth, AppError> {
        let health = self.request(&self.endpoint(&["health"])?, None, REQUEST_TIMEOUT)?;
        if health["status"] != "ok" || health["healthy"].as_bool() == Some(false) {
            return Err(AppError::invalid(
                "connection",
                "OpenViking 未报告就绪，请检查服务状态",
            ));
        }
        let version = health["version"].as_str().filter(|s| !s.is_empty());
        let identity = if health["auth_mode"] == "dev" {
            None
        } else {
            if health["role"] == "root" {
                return Err(AppError::from_message(
                    "AUTH_FAILED",
                    "connection",
                    false,
                    "当前为 OpenViking root 管理密钥，无法读写笔记与记忆，请改用用户或管理员 API Key",
                ));
            }
            match (health["account_id"].as_str(), health["user_id"].as_str()) {
                (Some(account), Some(user)) if !account.is_empty() && !user.is_empty() => {
                    Some(format!("账户 {account}，用户 {user}"))
                }
                _ => {
                    return Err(AppError::from_message(
                        "AUTH_FAILED",
                        "connection",
                        false,
                        "OpenViking 可达，但未确认认证身份，请检查 API Key",
                    ));
                }
            }
        };
        Ok(MemoryHealth {
            healthy: true,
            version: version.map(str::to_owned),
            identity,
        })
    }

    fn upsert_daily_note(
        &self,
        relative_uri: &str,
        markdown: &str,
        tags: &[String],
    ) -> Result<String, AppError> {
        let uri = self.resource_uri(relative_uri);
        let mut body = json!({"uri": uri, "content": markdown, "mode": "replace", "wait": false});
        let tags: Vec<_> = tags
            .iter()
            .filter_map(|tag| {
                let (key, value) = tag.split_once('=')?;
                (!key.is_empty()
                    && !value.is_empty()
                    && !value.contains('=')
                    && !tag.chars().any(char::is_whitespace))
                .then(|| tag.to_lowercase())
            })
            .collect();
        if !tags.is_empty() {
            body["tags"] = json!(tags);
        }
        let result = self.post(&["api", "v1", "content", "write"], body, REQUEST_TIMEOUT)?;
        Ok(result["uri"].as_str().unwrap_or(&uri).to_owned())
    }

    fn commit_note_memory(
        &self,
        _title: &str,
        markdown: &str,
        _tags: &[String],
    ) -> Result<Option<String>, AppError> {
        let created = self.post(&["api", "v1", "sessions"], json!({}), REQUEST_TIMEOUT)?;
        let session = created["session_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(invalid_response)?;
        self.post(
            &["api", "v1", "sessions", session, "messages"],
            json!({"role": "user", "content": markdown}),
            REQUEST_TIMEOUT,
        )?;
        let result = self.post(
            &["api", "v1", "sessions", session, "commit"],
            json!({}),
            REQUEST_TIMEOUT,
        )?;
        Ok(result["task_id"].as_str().map(str::to_owned))
    }

    fn search(
        &self,
        query: &str,
        limit: usize,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<Vec<MemoryHit>, AppError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut body = json!({
            "query": query,
            "limit": limit,
            "mode": "list",
            "context_type": ["memory", "resource"],
            "read_content": true,
        });
        if let Some(since) = since {
            body["since"] = json!(since);
        }
        if let Some(until) = until {
            body["until"] = json!(until);
        }
        let result = self.post(&["api", "v1", "search", "search"], body, SEARCH_TIMEOUT)?;
        if !result.is_object() {
            return Err(invalid_response());
        }
        let terms = extract_query_terms(query, "");
        let mut hits = Vec::new();
        for (field, kind) in [("memories", "ov_memory"), ("resources", "ov_resource")] {
            if let Some(items) = result[field].as_array() {
                hits.extend(
                    items
                        .iter()
                        .filter_map(|item| search_hit(item, kind, &terms)),
                );
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .unwrap_or(f64::NEG_INFINITY)
                .total_cmp(&a.score.unwrap_or(f64::NEG_INFINITY))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    fn read_content(&self, uri: &str) -> Result<String, AppError> {
        if !uri.starts_with("viking://") {
            return Err(AppError::invalid("memory", "OpenViking 引用地址无效"));
        }
        match self.read("read", uri) {
            Err(error) if error.code == "NOT_FOUND" => self.read("overview", uri),
            result => result,
        }
    }
}

fn unpack(mut envelope: Value) -> Result<Value, AppError> {
    match envelope["status"].as_str() {
        Some("ok") => envelope
            .get_mut("result")
            .map(Value::take)
            .ok_or_else(invalid_response),
        Some("error") => {
            let error = match envelope["error"]["code"].as_str() {
                Some("UNAUTHENTICATED" | "PERMISSION_DENIED") => AppError::from_message(
                    "AUTH_FAILED",
                    "memory",
                    false,
                    "OpenViking 认证失败或权限不足，请检查 API Key",
                ),
                Some("NOT_FOUND") => {
                    AppError::from_message("NOT_FOUND", "memory", false, "OpenViking 内容不存在")
                }
                code => AppError::from_message(
                    "SERVICE_ERROR",
                    "memory",
                    matches!(
                        code,
                        Some(
                            "UNAVAILABLE"
                                | "DEADLINE_EXCEEDED"
                                | "RESOURCE_EXHAUSTED"
                                | "CONFLICT"
                                | "INTERNAL"
                                | "INTERNAL_ERROR"
                        )
                    ),
                    "OpenViking 未能完成请求，请检查服务状态",
                ),
            };
            Err(error)
        }
        _ => Err(invalid_response()),
    }
}

fn invalid_response() -> AppError {
    AppError::invalid("memory", "OpenViking 响应格式无效，请核对服务地址和版本")
}

pub(crate) fn markdown_body(markdown: &str) -> &str {
    let text = markdown.trim();
    if let Some((first, mut rest)) = text.split_once('\n')
        && first.trim() == "---"
    {
        for line in rest.split_inclusive('\n') {
            rest = &rest[line.len()..];
            if matches!(line.trim(), "---" | "...") {
                return rest.trim_start();
            }
        }
    }
    text
}

pub(crate) fn memory_item_title(uri: &str, body: &str, abstract_text: &str) -> String {
    [body, abstract_text]
        .into_iter()
        .map(note_title)
        .find(|title| !title.is_empty() && !matches!(title.as_str(), "---" | "***" | "___"))
        .unwrap_or_else(|| {
            let name = uri.trim_end_matches('/').rsplit('/').next().unwrap_or(uri);
            name.strip_suffix(".md").unwrap_or(name).to_owned()
        })
}

fn search_hit(item: &Value, kind: &str, terms: &[String]) -> Option<MemoryHit> {
    let uri = item["uri"]
        .as_str()
        .filter(|uri| uri.starts_with("viking://"))?;
    let content = markdown_body(item["content"].as_str().unwrap_or_default());
    let abstract_text = markdown_body(item["abstract"].as_str().unwrap_or_default());
    let title = memory_item_title(uri, content, abstract_text);
    let snippet = if content.is_empty() {
        abstract_text.chars().take(480).collect()
    } else {
        focus_snippet(content, terms)
    };
    Some(MemoryHit {
        uri: uri.to_owned(),
        kind: kind.to_owned(),
        title,
        snippet,
        score: item["score"].as_f64(),
        updated_at: item["updated_at"].as_str().map(str::to_owned),
    })
}
