use serde_json::json;

use crate::dto::AppError;
use crate::dto::wikra::*;

use super::documents::in_directory;
use super::inbox::InboxCompilation;
use super::markdown::Markdown;
use super::*;

pub(crate) mod apply;
mod buffer;
pub(super) mod edit_ops;
mod recall;
pub(super) mod route;

pub(crate) use buffer::{BufferBatch, CaptureMode, schedule_capture};
pub(crate) use edit_ops::CompilationOutput;
pub(crate) use recall::{generate_teachback, local_teachback};

const STAGE: &str = "wikra_organize";

pub(crate) struct Source {
    pub(super) title: String,
    pub(super) path: String,
    pub(super) text: String,
}

pub(crate) struct ModelAccess {
    url: tauri::Url,
    pub(crate) alias: String,
    token: Option<String>,
}

impl ModelAccess {
    pub(crate) fn load(storage: &Storage) -> Result<Option<Self>, AppError> {
        let config = storage
            .model_config()
            .map_err(|_| AppError::invalid(STAGE, "无法读取模型配置，请在设置中重新保存"))?;
        let Some((endpoint, alias, _)) = config.filter(|config| config.2) else {
            return Ok(None);
        };
        let url = crate::model::chat_endpoint(&endpoint)
            .map_err(|message| AppError::invalid(STAGE, message))?;
        let token =
            crate::credential_commands::model_token(storage, crate::model::is_local_or_lan(&url))?;
        Ok(Some(Self { url, alias, token }))
    }

    fn generate<T: serde::de::DeserializeOwned>(
        &self,
        prompt: &str,
        input: serde_json::Value,
    ) -> Result<T, AppError> {
        let output =
            crate::model::generate(&self.url, &self.alias, self.token.as_deref(), prompt, input)
                .map_err(|_| {
                    AppError::from_message(
                        "MODEL_FAILED",
                        STAGE,
                        true,
                        "模型未完成整理，请检查连接或模型设置后重试",
                    )
                })?;
        serde_json::from_value(output)
            .map_err(|_| AppError::invalid(STAGE, "模型返回格式不符合要求，请重试"))
    }
}

#[derive(Clone, Copy)]
enum Compilation<'a> {
    Inbox(&'a InboxCompilation),
    Buffer(&'a BufferBatch),
}

fn db(err: rusqlite::Error) -> StorageError {
    database_error(STAGE, None, err)
}

fn tidy(text: &str, limit: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

fn inline_text(text: &str, limit: usize) -> String {
    tidy(text, limit)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace("%%", "&#37;&#37;")
}

fn citation_reference(path: &str, title: &str) -> String {
    let target = citation_target(path);
    let label = inline_text(title, 120).replace(['[', ']'], "");
    format!("[{label}](<{target}>)")
}

fn citation_target(path: &str) -> String {
    let (path, anchor) = path
        .split_once(NOTE_ANCHOR)
        .map_or((path, None), |(path, anchor)| (path, Some(anchor)));
    let mut target = path
        .replace('%', "%25")
        .replace('#', "%23")
        .replace(' ', "%20")
        .replace('[', "%5B")
        .replace(']', "%5D");
    if let Some(anchor) = anchor {
        target.push_str(NOTE_ANCHOR);
        target.push_str(anchor);
    }
    target
}

fn render_synthesis(title: &str) -> String {
    format!(
        "---\ntype: synthesis\nwikra-managed: true\n---\n\n# {}\n",
        inline_text(title, 120)
    )
}

impl Storage {
    pub(crate) fn list_organize_overview(&self) -> Result<OrganizeOverviewDto, StorageError> {
        let changesets = apply::list_changesets(&self.connection)?;
        let dismissed = self.dismissed_ids(true)?;
        let mut suggestions = Vec::new();
        for suggestion in self.list_wiki_suggestions()? {
            let id = format!("review:{}", suggestion.id);
            if !matches!(suggestion.status.as_str(), "pending" | "conflict")
                || dismissed.contains(&id)
            {
                continue;
            }
            suggestions.push(OrganizeSuggestionDto {
                id,
                kind: "review",
                kind_label: "待审改动",
                title: format!("审阅《{}》的改动", suggestion.target_path),
                detail: suggestion.reason,
                action_label: "查看知识页",
                page_path: suggestion.target_path,
            });
        }
        for node in self
            .get_knowledge_graph(false)?
            .nodes
            .into_iter()
            .filter(|node| node.is_orphan)
        {
            let id = format!("orphan:{}", node.id);
            if !dismissed.contains(&id) {
                suggestions.push(OrganizeSuggestionDto {
                    id,
                    kind: "orphan",
                    kind_label: "知识孤页",
                    title: node.title,
                    detail: "这页还没有双链或引文，可以补充相关页或原文。".to_owned(),
                    action_label: "查看知识页",
                    page_path: node.id,
                });
            }
        }
        Ok(OrganizeOverviewDto {
            conflicts: self.list_conflicts()?,
            changesets,
            suggestions,
            buffer_queue: self.buffer_queue()?,
            buffer_seconds_left: self.buffer_seconds_left()?,
        })
    }

    pub(crate) fn dismiss_suggestion(&self, id: &str) -> Result<(), StorageError> {
        self.dismiss_wikra_item(id, true)
    }

    pub(crate) fn dismiss_graph_gap(&self, id: &str) -> Result<(), StorageError> {
        self.dismiss_wikra_item(id, false)
    }

    fn dismiss_wikra_item(&self, id: &str, suggestion: bool) -> Result<(), StorageError> {
        if id.trim().is_empty() {
            return Err(invalid(STAGE, "未找到要忽略的项目"));
        }
        let sql = if suggestion {
            "INSERT OR IGNORE INTO dismissed_suggestions VALUES (?1)"
        } else {
            "INSERT OR IGNORE INTO dismissed_graph_gaps VALUES (?1)"
        };
        self.connection.execute(sql, [id]).map_err(db)?;
        Ok(())
    }

    pub(super) fn dismissed_ids(&self, suggestion: bool) -> Result<HashSet<String>, StorageError> {
        let sql = if suggestion {
            "SELECT id FROM dismissed_suggestions"
        } else {
            "SELECT id FROM dismissed_graph_gaps"
        };
        let mut statement = self.connection.prepare(sql).map_err(db)?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)
    }
}

fn set_compiled(
    conn: &Connection,
    id: &str,
    positions: &[i64],
    compiled: bool,
) -> Result<(), StorageError> {
    let mut statement = conn
        .prepare("UPDATE inbox_paragraphs SET compiled = ?3 WHERE item_id = ?1 AND position = ?2")
        .map_err(db)?;
    for position in positions {
        statement
            .execute(params![id, position, compiled])
            .map_err(db)?;
    }
    Ok(())
}
