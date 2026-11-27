use crate::dto::wikra::{InboxItemDetailDto, InboxItemSummaryDto, InboxParagraphDto};
use crate::platform::parse_http_url;

use super::markdown::{CodeFence, Markdown, heading};
use super::organize::Source;
use super::*;

mod web;

pub(super) const STAGE: &str = "wikra_inbox";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompileMode {
    Excerpt,
    Full,
    Archive,
}

impl CompileMode {
    pub(crate) fn parse(mode: &str) -> Result<Self, StorageError> {
        match mode {
            "excerpt" => Ok(Self::Excerpt),
            "full" => Ok(Self::Full),
            "archive" => Ok(Self::Archive),
            _ => Err(invalid(STAGE, "无效的收件箱整理方式")),
        }
    }
}

pub(crate) struct IngestedClip {
    title: String,
    domain: String,
    url: Option<String>,
    body: String,
    full: bool,
}

pub(crate) fn prepare_clip(
    input: String,
    title_hint: Option<String>,
) -> Result<IngestedClip, StorageError> {
    if input.trim().is_empty() {
        return Err(invalid(STAGE, "请输入网址或要收藏的正文"));
    }
    let title_hint = title_hint
        .as_deref()
        .map(one_line)
        .filter(|title| !title.is_empty());
    let trimmed = input.trim();
    if (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
        && !trimmed.contains(char::is_whitespace)
    {
        let url = parse_http_url(trimmed)
            .ok_or_else(|| invalid(STAGE, "请输入有效的 HTTP 或 HTTPS 网址"))?;
        let domain = url.host_str().expect("validated HTTP host").to_owned();
        let article = web::fetch(&url);
        let full = article.is_some();
        let title = title_hint
            .or_else(|| {
                article.as_ref().and_then(|article| {
                    if article.title.is_empty() {
                        text_title(&article.body)
                    } else {
                        Some(one_line(&article.title))
                    }
                })
            })
            .unwrap_or_else(|| domain.clone());
        let body = article.map(|article| article.body).unwrap_or_else(|| {
            format!(
                "# {}\n\n原网页：<{}>\n\n正文未获取，可在收件箱粘贴补全。\n",
                title.replace(['<', '>'], ""),
                url,
            )
        });
        return Ok(IngestedClip {
            title,
            domain,
            url: Some(url.into()),
            body,
            full,
        });
    }
    let title = title_hint
        .or_else(|| text_title(&input))
        .unwrap_or_else(|| "收藏原文".to_owned());
    Ok(IngestedClip {
        title,
        domain: "本地文本".to_owned(),
        url: None,
        body: input,
        full: true,
    })
}

fn one_line(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(120)
        .collect()
}

fn text_title(body: &str) -> Option<String> {
    let parsed = Markdown::parse(body);
    parsed
        .value("title")
        .or(parsed.title.as_deref())
        .map(one_line)
        .or_else(|| {
            body[parsed.content_start..]
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(one_line)
        })
}

fn slug(title: &str) -> String {
    let mut slug = String::new();
    for ch in title.chars().take(42) {
        if ch.is_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "clip".into()
    } else {
        slug.into()
    }
}

struct Paragraph {
    heading: String,
    text: String,
}

fn paragraphs(body: &str) -> Vec<Paragraph> {
    let parsed = Markdown::parse(body);
    let mut result = Vec::new();
    let mut current_heading = parsed
        .value("title")
        .or(parsed.title.as_deref())
        .unwrap_or("")
        .to_owned();
    let mut block = Vec::new();
    let mut fence = CodeFence::default();
    let flush = |block: &mut Vec<&str>, heading: &str, result: &mut Vec<Paragraph>| {
        if !block.is_empty() {
            let text = block.join("\n");
            block.clear();
            if !text.trim().is_empty() {
                result.push(Paragraph {
                    heading: heading.to_owned(),
                    text,
                });
            }
        }
    };
    for line in body[parsed.content_start..].lines() {
        let in_code = fence.contains(line);
        if !in_code && let Some((_, title)) = heading(line) {
            flush(&mut block, &current_heading, &mut result);
            current_heading = title.to_owned();
        } else if !in_code && line.trim().is_empty() {
            flush(&mut block, &current_heading, &mut result);
        } else {
            block.push(line);
        }
    }
    flush(&mut block, &current_heading, &mut result);
    if result.is_empty() && !body.trim().is_empty() {
        result.push(Paragraph {
            heading: current_heading,
            text: body.trim().to_owned(),
        });
    }
    result
}

fn word_count(body: &str) -> usize {
    let mut in_word = false;
    let mut words = 0;
    for ch in body.chars() {
        let cjk = matches!(ch, '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}'
            | '\u{ac00}'..='\u{d7af}' | '\u{20000}'..='\u{323af}');
        if cjk || ch.is_alphanumeric() && !in_word {
            words += 1;
        }
        in_word = ch.is_alphanumeric() && !cjk;
    }
    words
}

const ITEM_SUMMARY: &str =
    "SELECT i.id, i.title, i.domain, i.source_type, i.url, i.raw_path, i.saved_at,
            i.fetch_status, i.state, i.word_count,
            (SELECT count(*) FROM inbox_paragraphs p WHERE p.item_id = i.id AND p.highlighted = 1)
     FROM inbox_items i";

fn db(err: rusqlite::Error) -> StorageError {
    database_error(STAGE, None, err)
}

fn map_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<InboxItemSummaryDto> {
    let words: u32 = row.get(9)?;
    let fetch_status: String = row.get(7)?;
    Ok(InboxItemSummaryDto {
        id: row.get(0)?,
        title: row.get(1)?,
        domain: row.get(2)?,
        source_type: row.get(3)?,
        url: row.get(4)?,
        raw_path: row.get(5)?,
        saved_at: row.get(6)?,
        read_time: if fetch_status == "stub" {
            "待补全文".to_owned()
        } else {
            format!("{} 分钟", words.div_ceil(300).max(1))
        },
        fetch_status,
        state: row.get(8)?,
        word_count: words,
        highlight_count: row.get(10)?,
    })
}

fn item_summary(db_conn: &Connection, id: &str) -> Result<InboxItemSummaryDto, StorageError> {
    db_conn
        .query_row(
            &format!("{ITEM_SUMMARY} WHERE i.id = ?1"),
            [id],
            map_summary,
        )
        .optional()
        .map_err(db)?
        .ok_or_else(|| invalid(STAGE, "找不到这份收件箱原文"))
}

struct SavedParagraph {
    position: i64,
    heading: String,
    text: String,
    highlighted: bool,
    compiled: bool,
}

fn item_paragraphs(db_conn: &Connection, id: &str) -> Result<Vec<SavedParagraph>, StorageError> {
    let mut statement = db_conn.prepare(
        "SELECT position, heading, text, highlighted, compiled FROM inbox_paragraphs WHERE item_id = ?1 ORDER BY position",
    ).map_err(db)?;
    statement
        .query_map([id], |row| {
            Ok(SavedParagraph {
                position: row.get(0)?,
                heading: row.get(1)?,
                text: row.get(2)?,
                highlighted: row.get(3)?,
                compiled: row.get(4)?,
            })
        })
        .map_err(db)?
        .collect::<Result<_, _>>()
        .map_err(db)
}

fn paragraph_id(id: &str, position: i64) -> String {
    format!("{id}:{position}")
}

fn insert_paragraphs(
    conn: &Connection,
    id: &str,
    paragraphs: &[Paragraph],
) -> Result<(), StorageError> {
    let mut statement = conn.prepare("INSERT INTO inbox_paragraphs(item_id, position, heading, text) VALUES (?1, ?2, ?3, ?4)")
        .map_err(db)?;
    for (position, paragraph) in paragraphs.iter().enumerate() {
        statement
            .execute(params![
                id,
                position as i64,
                paragraph.heading,
                paragraph.text
            ])
            .map_err(db)?;
    }
    Ok(())
}

pub(crate) struct InboxCompilation {
    pub(super) id: String,
    pub(super) title: String,
    raw_path: String,
    expected_hash: String,
    state: String,
    mode: CompileMode,
    pub(super) positions: Vec<i64>,
    pub(crate) sources: Vec<Source>,
}

impl InboxCompilation {
    pub(super) fn verify(
        &self,
        conn: &Connection,
        files: &WorkspaceFiles,
    ) -> Result<(), StorageError> {
        let summary = item_summary(conn, &self.id)?;
        let conflicted = || conflict(STAGE, "收件箱原文、存档状态或选段已有变化，请刷新后重试");
        if summary.fetch_status != "full" || summary.state == "archived" && self.state != "archived"
        {
            return Err(conflicted());
        }
        let bytes = files
            .read_markdown(&self.raw_path)
            .map_err(|err| source_error(STAGE, err))?;
        if revision_hash(&bytes) != self.expected_hash {
            return Err(conflicted());
        }
        let paragraphs = item_paragraphs(conn, &self.id)?;
        for position in &self.positions {
            if !paragraphs.iter().any(|p| {
                p.position == *position
                    && !p.compiled
                    && (self.mode == CompileMode::Full || p.highlighted)
            }) {
                return Err(conflicted());
            }
        }
        Ok(())
    }
}

impl Storage {
    pub(crate) fn list_inbox_items(&self) -> Result<Vec<InboxItemSummaryDto>, StorageError> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "{ITEM_SUMMARY} ORDER BY i.saved_at DESC, i.rowid DESC"
            ))
            .map_err(db)?;
        statement
            .query_map([], map_summary)
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)
    }

    pub(crate) fn inbox_item(&self, id: &str) -> Result<InboxItemDetailDto, StorageError> {
        let summary = item_summary(&self.connection, id)?;
        let paragraphs = item_paragraphs(&self.connection, id)?
            .into_iter()
            .map(|p| InboxParagraphDto {
                id: paragraph_id(id, p.position),
                heading: p.heading,
                text: p.text,
                highlighted: p.highlighted,
                compiled: p.compiled,
            })
            .collect::<Vec<_>>();
        let summary_points = paragraphs
            .iter()
            .take(3)
            .map(|p| {
                p.text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(160)
                    .collect()
            })
            .collect();
        let mut pages = self.connection.prepare(
            "SELECT DISTINCT target_path FROM wikra_changesets WHERE inbox_id = ?1 AND status = 'applied' ORDER BY target_path",
        ).map_err(db)?;
        let related_page_paths = pages
            .query_map([id], |row| row.get(0))
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)?;
        Ok(InboxItemDetailDto {
            summary,
            summary_points,
            related_page_paths,
            paragraphs,
        })
    }

    pub(crate) fn open_inbox_item(&self, id: &str) -> Result<InboxItemDetailDto, StorageError> {
        self.connection
            .execute(
                "UPDATE inbox_items SET state = 'reading' WHERE id = ?1 AND state = 'unread'",
                [id],
            )
            .map_err(db)?;
        self.inbox_item(id)
    }

    pub(crate) fn create_inbox_item(
        &mut self,
        clip: IngestedClip,
    ) -> Result<InboxItemDetailDto, StorageError> {
        let clips_dir = self.clips_dir();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let (id, time) = database_identity(&tx, STAGE)?;
        let month: String = tx
            .query_row("SELECT strftime('%Y/%m','now','localtime')", [], |row| {
                row.get(0)
            })
            .map_err(db)?;
        let relative_path = format!("{clips_dir}/{month}/{}-{}.md", slug(&clip.title), &id[..12]);
        let edit = MarkdownEdit::load(&self.workspace.files, &relative_path, true, STAGE)?;
        if !documents::in_directory(&edit.key, &clips_dir) || edit.before.is_some() {
            return Err(conflict(
                STAGE,
                "原文目录已被移动或文件已存在，请重新选择工作区",
            ));
        }
        let parts = if clip.full {
            paragraphs(&clip.body)
        } else {
            Vec::new()
        };
        let words = parts
            .iter()
            .map(|p| word_count(&p.text) as i64)
            .sum::<i64>();
        tx.execute(
            "INSERT INTO inbox_items(id, title, domain, source_type, url, raw_path, raw_hash, saved_at, fetch_status, state, word_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'unread', ?10)",
            params![id, clip.title, clip.domain, if clip.url.is_some() { "web" } else { "text" },
                clip.url, edit.key, revision_hash(clip.body.as_bytes()), time, if clip.full { "full" } else { "stub" }, words],
        ).map_err(db)?;
        insert_paragraphs(&tx, &id, &parts)?;
        edit.commit(tx, Some(&clip.body), STAGE)?;
        self.inbox_item(&id)
    }

    pub(crate) fn complete_inbox_stub(
        &mut self,
        id: &str,
        pasted_body: &str,
    ) -> Result<InboxItemDetailDto, StorageError> {
        if pasted_body.trim().is_empty() {
            return Err(invalid(STAGE, "请粘贴要补全的正文"));
        }
        let parts = paragraphs(pasted_body);
        let words = parts
            .iter()
            .map(|p| word_count(&p.text) as i64)
            .sum::<i64>();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let item = item_summary(&tx, id)?;
        if item.fetch_status != "stub" {
            return Err(conflict(STAGE, "原文已经完整，请刷新查看"));
        }
        let expected_hash: String = tx
            .query_row(
                "SELECT raw_hash FROM inbox_items WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .map_err(db)?;
        let edit = MarkdownEdit::load(&self.workspace.files, &item.raw_path, false, STAGE)?;
        if edit.key != item.raw_path
            || edit
                .before
                .as_ref()
                .is_none_or(|text| revision_hash(text.as_bytes()) != expected_hash)
        {
            return Err(conflict(STAGE, "原文收据已在外部被修改，请刷新后重试"));
        }
        tx.execute(
            "UPDATE inbox_items SET fetch_status = 'full', raw_hash = ?2, word_count = ?3,
                    state = CASE WHEN state = 'unread' THEN 'reading' ELSE state END WHERE id = ?1",
            params![id, revision_hash(pasted_body.as_bytes()), words],
        )
        .map_err(db)?;
        insert_paragraphs(&tx, id, &parts)?;
        edit.commit(tx, Some(pasted_body), STAGE)?;
        self.inbox_item(id)
    }

    pub(crate) fn toggle_inbox_paragraph(
        &self,
        id: &str,
        paragraph: &str,
    ) -> Result<InboxItemDetailDto, StorageError> {
        let position = paragraph
            .split_once(':')
            .filter(|(owner, _)| *owner == id)
            .and_then(|(_, position)| position.parse::<i64>().ok())
            .ok_or_else(|| invalid(STAGE, "找不到这个原文段落"))?;
        let updated = self
            .connection
            .execute(
                "UPDATE inbox_paragraphs SET highlighted = 1 - highlighted
             WHERE item_id = ?1 AND position = ?2 AND compiled = 0",
                params![id, position],
            )
            .map_err(db)?;
        if updated == 0 {
            return Err(invalid(
                STAGE,
                "段落不存在或已整理，如需修改请先撤销本次整理",
            ));
        }
        self.open_inbox_item(id)
    }

    pub(crate) fn archive_inbox_item(&self, id: &str) -> Result<InboxItemDetailDto, StorageError> {
        self.connection
            .execute(
                "UPDATE inbox_items SET state = 'archived' WHERE id = ?1",
                [id],
            )
            .map_err(db)?;
        self.inbox_item(id)
    }

    pub(crate) fn prepare_inbox_compile(
        &self,
        id: &str,
        mode: CompileMode,
    ) -> Result<Option<InboxCompilation>, StorageError> {
        let item = item_summary(&self.connection, id)?;
        if item.fetch_status == "stub" {
            return Err(invalid(STAGE, "请先在收件箱补全原文再整理"));
        }
        let selected = item_paragraphs(&self.connection, id)?
            .into_iter()
            .filter(|p| !p.compiled && (mode == CompileMode::Full || p.highlighted))
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return if mode == CompileMode::Full {
                Ok(None)
            } else {
                Err(invalid(STAGE, "请先标记尚未整理的重点段落"))
            };
        }
        let expected_hash: String = self
            .connection
            .query_row(
                "SELECT raw_hash FROM inbox_items WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .map_err(db)?;
        let body = String::from_utf8(
            self.workspace
                .files
                .read_markdown(&item.raw_path)
                .map_err(|err| source_error(STAGE, err))?,
        )
        .map_err(|err| source_error(STAGE, err))?;
        if revision_hash(body.as_bytes()) != expected_hash {
            return Err(conflict(
                STAGE,
                "收件箱原文已在外部被修改，请刷新后重新收藏",
            ));
        }
        let sources = if mode == CompileMode::Full {
            vec![Source {
                title: item.title.clone(),
                path: item.raw_path.clone(),
                text: body,
            }]
        } else {
            selected
                .iter()
                .map(|p| Source {
                    title: if p.heading.is_empty() {
                        item.title.clone()
                    } else {
                        p.heading.clone()
                    },
                    path: item.raw_path.clone(),
                    text: p.text.clone(),
                })
                .collect()
        };
        Ok(Some(InboxCompilation {
            id: id.to_owned(),
            title: item.title,
            raw_path: item.raw_path,
            expected_hash,
            state: item.state,
            mode,
            positions: selected.into_iter().map(|p| p.position).collect(),
            sources,
        }))
    }
}
