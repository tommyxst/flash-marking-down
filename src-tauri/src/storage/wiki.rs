use std::collections::HashMap;
use std::ops::Range;

use crate::dto::wikra::DocTaskItemDto;
use crate::dto::{TaskEntryDto, WikiDocumentDto, WikiSuggestionDto};

use super::*;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS wiki_targets (
    path TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('wiki','tasks'))
);
CREATE UNIQUE INDEX IF NOT EXISTS one_task_list ON wiki_targets(kind) WHERE kind='tasks';
CREATE TABLE IF NOT EXISTS wiki_suggestions (
    id TEXT PRIMARY KEY,
    note_id TEXT,
    target_path TEXT NOT NULL,
    base_hash TEXT NOT NULL,
    before_text TEXT NOT NULL,
    after_text TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL,
    created_at TEXT NOT NULL
);";

pub(crate) type WikiApplyOutcome = (String, String, String);

pub(crate) const CAPTURE_TARGET_MESSAGE: &str = "随手记目录内的文件不能作为 Wiki 或任务清单目标";

fn db(e: rusqlite::Error) -> StorageError {
    database_error("wiki", None, e)
}

fn invalid(message: &'static str) -> StorageError {
    error("INVALID_INPUT", "wiki", None, false, message, message)
}

fn conflict(message: &'static str) -> StorageError {
    error("VERSION_CONFLICT", "wiki", None, true, message, message)
}

fn file_error(e: io::Error) -> StorageError {
    let message = if crate::platform::is_sharing_violation(&e) {
        "文件正被其他程序占用，请保存或关闭后重新比较"
    } else {
        "无法访问目标 Markdown，请检查路径、权限或文件占用"
    };
    error("SOURCE_UNAVAILABLE", "wiki", None, true, message, e)
}

fn utf8(bytes: Vec<u8>) -> Result<String, StorageError> {
    String::from_utf8(bytes).map_err(|_| invalid("目标文件不是有效的 UTF-8 Markdown"))
}

pub(super) fn update_wiki_index(
    conn: &Connection,
    path: &str,
    body: &str,
) -> Result<(), StorageError> {
    let captured_at: Option<String> = conn
        .query_row(
            "SELECT captured_at FROM search_documents WHERE relative_path = ?1",
            [path],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(db)?
        .flatten();
    search::upsert_document(
        conn,
        path,
        captured_at.as_deref(),
        &revision_hash(body.as_bytes()),
        body,
    )
    .map_err(db)
}

fn map_suggestion_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WikiSuggestionDto> {
    Ok(WikiSuggestionDto {
        id: row.get(0)?,
        note_id: row.get(1)?,
        target_path: row.get(2)?,
        base_hash: row.get(3)?,
        before_text: row.get(4)?,
        after_text: row.get(5)?,
        reason: row.get(6)?,
        status: row.get(7)?,
        created_at: row.get(8)?,
    })
}

fn load_suggestion(conn: &Connection, id: &str) -> Result<WikiSuggestionDto, StorageError> {
    conn.query_row(
        "SELECT id, note_id, target_path, base_hash, before_text, after_text, reason, status, created_at
         FROM wiki_suggestions WHERE id = ?1",
        [id],
        map_suggestion_row,
    )
    .optional()
    .map_err(db)?
    .ok_or_else(|| invalid("找不到对应的 Wiki 变更建议"))
}

fn append_markdown_section(before: &str, content: &str) -> String {
    let line_break = if before.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let normalized = content
        .trim()
        .replace("\r\n", "\n")
        .replace('\n', line_break);
    let mut after = before.to_owned();
    if !after.is_empty() && !after.ends_with('\n') {
        after.push_str(line_break);
    }
    if !after.is_empty() {
        after.push_str(line_break);
    }
    after.push_str(&normalized);
    after.push_str(line_break);
    after
}

impl Storage {
    pub(crate) fn create_wiki_target(&mut self, path: &str) -> Result<(), StorageError> {
        let (key, target) = self
            .workspace
            .files
            .create_markdown_target(path)
            .map_err(file_error)?;
        if self.is_in_capture_dir(&key) || self.is_capture_path(&key) {
            return Err(invalid(CAPTURE_TARGET_MESSAGE));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute(
            "INSERT INTO wiki_targets(path,kind) VALUES (?1,'wiki') ON CONFLICT(path) DO NOTHING",
            [&key],
        )
        .map_err(db)?;
        let role: String = tx
            .query_row("SELECT kind FROM wiki_targets WHERE path=?1", [&key], |r| {
                r.get(0)
            })
            .map_err(db)?;
        if role != "wiki" {
            return Err(invalid("此路径已映射为任务清单"));
        }
        update_wiki_index(&tx, &key, "")?;
        replace_markdown(&target, None, &[]).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                conflict("同名文件已存在，请使用关联已有页")
            } else {
                file_error(e)
            }
        })?;
        tx.commit().map_err(db)
    }

    pub(crate) fn wiki_targets(&self) -> Result<Vec<(String, String)>, StorageError> {
        let mut q = self
            .connection
            .prepare("SELECT kind,path FROM wiki_targets ORDER BY kind,path")
            .map_err(db)?;
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)
    }

    pub(crate) fn list_wiki_documents(&self) -> Result<Vec<WikiDocumentDto>, StorageError> {
        let mut mapped_wiki = HashSet::new();
        let mut tasks_target = None;
        for (kind, path) in self.wiki_targets()? {
            match kind.as_str() {
                "wiki" => {
                    mapped_wiki.insert(path);
                }
                "tasks" => tasks_target = Some(path),
                _ => (),
            }
        }
        let mut counts = self
            .connection
            .prepare(
                "SELECT target_path, SUM(status = 'pending'), SUM(status = 'conflict')
             FROM wiki_suggestions WHERE status IN ('pending', 'conflict') GROUP BY target_path",
            )
            .map_err(db)?;
        let counts = counts
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (
                        row.get::<_, i64>(1)? as usize,
                        row.get::<_, i64>(2)? as usize,
                    ),
                ))
            })
            .map_err(db)?
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(db)?;
        let capture_files = self.capture_paths()?;
        let (entries, _) = self
            .workspace
            .files
            .markdown_entries()
            .map_err(file_error)?;
        let mut timestamp = self
            .connection
            .prepare("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', ?1, 'unixepoch')")
            .map_err(db)?;
        let mut documents = Vec::new();
        for entry in entries {
            let path = entry.relative_path;
            if self.is_in_capture_dir(&path)
                || capture_files.contains(&path)
                || tasks_target.as_deref() == Some(path.as_str())
            {
                continue;
            }
            let Ok(bytes) = self.workspace.files.read_markdown(&path) else {
                continue;
            };
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let mut title = note_title(text);
            if title.is_empty() {
                title = Path::new(&path)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&path)
                    .to_owned();
            }
            let folder = path
                .rsplit_once('/')
                .map_or(String::new(), |(dir, _)| dir.to_owned());
            let (pending_suggestions, conflict_suggestions) =
                counts.get(&path).copied().unwrap_or_default();
            let updated_at = if entry.mtime_ns > 0 {
                // MarkdownEntry stores Windows FILETIME ticks (100 ns since 1601).
                let unix_seconds = entry.mtime_ns / 10_000_000 - 11_644_473_600;
                timestamp
                    .query_row([unix_seconds], |row| row.get(0))
                    .map_err(db)?
            } else {
                None
            };
            documents.push(WikiDocumentDto {
                title,
                preview: note_preview(text),
                folder,
                mapped: mapped_wiki.contains(&path),
                pending_suggestions,
                conflict_suggestions,
                updated_at,
                path,
            });
        }
        documents.sort_unstable_by(|a, b| {
            (!a.mapped, &a.folder, &a.path).cmp(&(!b.mapped, &b.folder, &b.path))
        });
        Ok(documents)
    }

    pub(crate) fn map_wiki_target(&mut self, kind: &str, path: &str) -> Result<(), StorageError> {
        if !matches!(kind, "tasks" | "wiki") {
            return Err(invalid("请选择 Wiki 或任务清单"));
        }
        let (target, full_path) = self
            .workspace
            .files
            .markdown_file(path)
            .map_err(file_error)?;
        if self.is_in_capture_dir(&target) || self.is_capture_path(&target) {
            return Err(invalid(CAPTURE_TARGET_MESSAGE));
        }
        wikra::ensure_editable(&self.connection, &target, "wiki")?;
        utf8(fs::read(full_path).map_err(file_error)?)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT kind FROM wiki_targets WHERE path=?1",
                [&target],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        if let Some(previous) = previous {
            return if previous == kind {
                Ok(())
            } else {
                Err(invalid("同一文件已映射为另一种用途"))
            };
        }
        if kind == "tasks" {
            tx.execute("DELETE FROM wiki_targets WHERE kind='tasks'", [])
                .map_err(db)?;
        }
        tx.execute(
            "INSERT INTO wiki_targets VALUES (?1,?2)",
            params![target, kind],
        )
        .map_err(db)?;
        tx.commit().map_err(db)
    }

    pub(crate) fn unmap_wiki_target(&mut self, path: &str) -> Result<(), StorageError> {
        let affected = self
            .connection
            .execute("DELETE FROM wiki_targets WHERE path = ?1", [path])
            .map_err(db)?;
        if affected == 0 {
            return Err(invalid("找不到该映射目标"));
        }
        Ok(())
    }

    fn check_wiki_target(&self, target: &str, kind: &str) -> Result<(), StorageError> {
        wikra::ensure_editable(&self.connection, target, "wiki")?;
        let mapped: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM wiki_targets WHERE path=?1 AND kind=?2)",
                params![target, kind],
                |r| r.get(0),
            )
            .map_err(db)?;
        if mapped {
            Ok(())
        } else {
            Err(invalid("该文件不在当前映射的写入目标中"))
        }
    }

    pub(crate) fn list_wiki_suggestions(&self) -> Result<Vec<WikiSuggestionDto>, StorageError> {
        let mut stmt = self
            .connection
            .prepare(
                "SELECT id, note_id, target_path, base_hash, before_text, after_text, reason, status, created_at
                 FROM wiki_suggestions
                 ORDER BY CASE status WHEN 'pending' THEN 0 WHEN 'conflict' THEN 1 ELSE 2 END, rowid DESC",
            )
            .map_err(db)?;
        stmt.query_map([], map_suggestion_row)
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)
    }

    pub(crate) fn wiki_suggestions_for_note(
        &self,
        note_id: &str,
    ) -> Result<Vec<WikiSuggestionDto>, StorageError> {
        let mut stmt = self
            .connection
            .prepare(
                "SELECT id, note_id, target_path, base_hash, before_text, after_text, reason, status, created_at
                 FROM wiki_suggestions
                 WHERE note_id = ?1
                 ORDER BY rowid DESC",
            )
            .map_err(db)?;
        stmt.query_map([note_id], map_suggestion_row)
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)
    }

    pub(crate) fn wiki_suggestions_for_target(
        &self,
        target_path: &str,
    ) -> Result<Vec<WikiSuggestionDto>, StorageError> {
        let mut stmt = self.connection.prepare(
            "SELECT id, note_id, target_path, base_hash, before_text, after_text, reason, status, created_at
             FROM wiki_suggestions WHERE target_path = ?1
             ORDER BY CASE status WHEN 'pending' THEN 0 WHEN 'conflict' THEN 1 ELSE 2 END, rowid DESC",
        ).map_err(db)?;
        stmt.query_map([target_path], map_suggestion_row)
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)
    }

    pub(crate) fn create_wiki_suggestion(
        &mut self,
        note_id: Option<&str>,
        target_path: &str,
        content: &str,
        reason: &str,
    ) -> Result<WikiSuggestionDto, StorageError> {
        let content = content.trim();
        if content.is_empty() {
            return Err(invalid("建议内容不能为空"));
        }
        let target = self
            .workspace
            .files
            .markdown_key(target_path)
            .map_err(file_error)?;
        self.check_wiki_target(&target, "wiki")?;

        let before = utf8(
            self.workspace
                .files
                .read_markdown(&target)
                .map_err(file_error)?,
        )?;
        let after = append_markdown_section(&before, content);
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM wiki_suggestions
                 WHERE note_id IS ?1 AND target_path = ?2 AND after_text = ?3
                   AND status IN ('pending', 'conflict', 'applied')
                 ORDER BY rowid DESC LIMIT 1",
                params![note_id, target, after],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        if let Some(id) = existing {
            return load_suggestion(&self.connection, &id);
        }

        let base_hash = revision_hash(before.as_bytes());
        let id: String = self
            .connection
            .query_row(
                "INSERT INTO wiki_suggestions (
                    id, note_id, target_path, base_hash, before_text, after_text, reason, status, created_at
                 ) VALUES (
                    lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, ?6, 'pending',
                    strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 ) RETURNING id",
                params![note_id, target, base_hash, before, after, reason.trim()],
                |r| r.get(0),
            )
            .map_err(db)?;
        load_suggestion(&self.connection, &id)
    }

    pub(crate) fn apply_wiki_suggestion(
        &mut self,
        id: &str,
    ) -> Result<WikiApplyOutcome, StorageError> {
        let item = load_suggestion(&self.connection, id)?;
        if item.status == "applied" {
            return Ok((item.id, "applied".into(), String::new()));
        }
        if item.status != "pending" {
            return Err(invalid("该建议当前无法采用，请先重新比较或刷新"));
        }
        self.check_wiki_target(&item.target_path, "wiki")?;
        let full_path = self
            .workspace
            .files
            .markdown_full_path(&item.target_path)
            .map_err(file_error)?;
        let current = fs::read(&full_path).map_err(file_error)?;
        if current != item.before_text.as_bytes() {
            self.connection
                .execute(
                    "UPDATE wiki_suggestions SET status = 'conflict' WHERE id = ?1",
                    [id],
                )
                .map_err(db)?;
            return Err(conflict("目标 Wiki 文件已被外部修改，请先重新比较"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        update_wiki_index(&tx, &item.target_path, &item.after_text)?;
        tx.execute(
            "UPDATE wiki_suggestions SET status = 'applied' WHERE id = ?1",
            [id],
        )
        .map_err(db)?;
        #[cfg(test)]
        tests::at_write_boundary();
        if let Err(e) = replace_markdown(&full_path, Some(&current), item.after_text.as_bytes()) {
            drop(tx);
            if e.kind() == io::ErrorKind::AlreadyExists {
                self.connection
                    .execute(
                        "UPDATE wiki_suggestions SET status = 'conflict' WHERE id = ?1",
                        [id],
                    )
                    .map_err(db)?;
                return Err(conflict("目标 Wiki 文件刚被其他编辑器修改，请重新比较"));
            }
            return Err(file_error(e));
        }
        tx.commit().map_err(db)?;
        Ok((item.id, "applied".into(), String::new()))
    }

    pub(crate) fn recompare_wiki_suggestion(
        &mut self,
        id: &str,
    ) -> Result<WikiSuggestionDto, StorageError> {
        let item = load_suggestion(&self.connection, id)?;
        if !matches!(item.status.as_str(), "pending" | "conflict") {
            return Err(invalid("该建议无需重新比较"));
        }
        self.check_wiki_target(&item.target_path, "wiki")?;
        let current = utf8(
            self.workspace
                .files
                .read_markdown(&item.target_path)
                .map_err(file_error)?,
        )?;
        let added = item
            .after_text
            .strip_prefix(&item.before_text)
            .unwrap_or(&item.after_text)
            .trim();
        let next_after = append_markdown_section(&current, added);
        let next_hash = revision_hash(current.as_bytes());
        self.connection
            .execute(
                "UPDATE wiki_suggestions
                 SET base_hash = ?2, before_text = ?3, after_text = ?4, status = 'pending'
                 WHERE id = ?1",
                params![id, next_hash, current, next_after],
            )
            .map_err(db)?;
        load_suggestion(&self.connection, id)
    }

    pub(crate) fn reject_wiki_suggestion(&mut self, id: &str) -> Result<(), StorageError> {
        let updated = self
            .connection
            .execute(
                "UPDATE wiki_suggestions SET status = 'rejected' WHERE id = ?1 AND status IN ('pending', 'conflict')",
                [id],
            )
            .map_err(db)?;
        if updated == 0 {
            return Err(invalid("该建议当前状态无法拒绝"));
        }
        Ok(())
    }

    pub(crate) fn undo_wiki_suggestion(
        &mut self,
        id: &str,
    ) -> Result<WikiSuggestionDto, StorageError> {
        let item = load_suggestion(&self.connection, id)?;
        if item.status != "applied" {
            return Err(invalid("仅支持撤销已采用的 Wiki 变更"));
        }
        self.check_wiki_target(&item.target_path, "wiki")?;
        let full_path = self
            .workspace
            .files
            .markdown_full_path(&item.target_path)
            .map_err(file_error)?;
        let current = utf8(fs::read(&full_path).map_err(file_error)?)?;
        let restored = if current == item.after_text {
            item.before_text.clone()
        } else {
            let added = item
                .after_text
                .strip_prefix(&item.before_text)
                .ok_or_else(|| conflict("无法定位原始新增段落，请手工核对"))?;
            let mut matches = current.match_indices(added);
            let Some((pos, _)) = matches.next() else {
                return Err(conflict("已采纳的段落被编辑，无法自动撤销，请手工核对"));
            };
            if added.is_empty() || matches.next().is_some() {
                return Err(conflict("目标文件存在多处相同内容，无法自动撤销"));
            }
            let mut next = current.clone();
            let remaining = &current[pos + added.len()..];
            let keep_break = pos > 0
                && !current[..pos].ends_with('\n')
                && !remaining.is_empty()
                && !remaining.starts_with(['\r', '\n'])
                && added.ends_with('\n');
            let sep = if keep_break && added.ends_with("\r\n") {
                "\r\n"
            } else if keep_break {
                "\n"
            } else {
                ""
            };
            next.replace_range(pos..pos + added.len(), sep);
            next
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        update_wiki_index(&tx, &item.target_path, &restored)?;
        tx.execute(
            "UPDATE wiki_suggestions SET status = 'reverted' WHERE id = ?1",
            [id],
        )
        .map_err(db)?;
        replace_markdown(&full_path, Some(current.as_bytes()), restored.as_bytes()).map_err(
            |e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    conflict("目标 Wiki 文件刚被外部修改，请重试撤销")
                } else {
                    file_error(e)
                }
            },
        )?;
        tx.commit().map_err(db)?;
        load_suggestion(&self.connection, id)
    }

    pub(crate) fn list_all_tasks(
        &self,
    ) -> Result<(String, String, Vec<TaskEntryDto>, Vec<String>), StorageError> {
        let mapped_target: Option<String> = self
            .connection
            .query_row(
                "SELECT path FROM wiki_targets WHERE kind='tasks'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;

        let mut entries = Vec::new();
        let mut notices = Vec::new();
        let mut revision = String::new();

        if let Some(target) = &mapped_target {
            if let Ok(bytes) = self.workspace.files.read_markdown(target)
                && let Ok(body) = String::from_utf8(bytes)
            {
                revision = revision_hash(body.as_bytes());
                let (anchored, manual, mut issues) = parse_tasks(&body);
                notices.append(&mut issues);
                for (id, list) in anchored {
                    let unique = list.len() == 1;
                    if !unique {
                        notices.push("任务清单存在重复锚点，请核对对应行".into());
                    }
                    for item in list {
                        let source_path = item.source_path.unwrap_or_default();
                        let source_is_capture =
                            !source_path.is_empty() && self.is_capture_path(&source_path);
                        entries.push(TaskEntryDto {
                            id: if unique { id.clone() } else { String::new() },
                            path: target.clone(),
                            expected_line: item.line,
                            text: item.text,
                            state: if unique { item.state } else { "invalid" }.into(),
                            source_path,
                            source_is_capture,
                            manual: false,
                            assignee: None,
                            due: None,
                            note_id: None,
                        });
                    }
                }
                for (line, item) in manual {
                    entries.push(TaskEntryDto {
                        id: format!("line:{line}"),
                        path: target.clone(),
                        expected_line: item.line,
                        text: item.text,
                        state: item.state.into(),
                        source_path: String::new(),
                        source_is_capture: false,
                        manual: true,
                        assignee: None,
                        due: None,
                        note_id: None,
                    });
                }
            }
        }

        // Also include checkboxes and organized tasks from notes (avoiding duplicate paths if mapped).
        let mut stmt = self
            .connection
            .prepare(
                "SELECT id, relative_path, block_id, organize_json
                 FROM notes
                 ORDER BY captured_at DESC, rowid DESC LIMIT 120",
            )
            .map_err(db)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(db)?;

        let mut file_cache: HashMap<String, Option<String>> = HashMap::new();
        for row in rows {
            let (note_id, path, block_id, organize_json) = row.map_err(db)?;
            let locator = note_locator(&path, block_id.as_deref());
            let org = organize_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<crate::dto::OrganizeResultDto>(raw).ok());

            let cached = file_cache.entry(path.clone()).or_insert_with(|| {
                self.workspace
                    .files
                    .read_markdown(&path)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
            });

            if mapped_target.as_deref() != Some(path.as_str())
                && let Some(full_text) = cached.as_deref()
                && let Some(lines) = task_lines(full_text, block_id.as_deref())
            {
                let (_, manual, _) = parse_tasks(full_text);
                for (file_line, task) in manual {
                    if !lines.contains(&file_line) {
                        continue;
                    }
                    let note_line = file_line - lines.start + 1;
                    let line_id = format!("line:{note_line}");
                    let meta = org.as_ref().and_then(|o| {
                        o.tasks
                            .iter()
                            .find(|t| t.line == Some(note_line) && t.text == task.text)
                    });
                    if !entries.iter().any(|e| e.path == locator && e.id == line_id) {
                        entries.push(TaskEntryDto {
                            id: line_id,
                            path: locator.clone(),
                            expected_line: task.line,
                            text: task.text,
                            state: task.state.into(),
                            source_path: locator.clone(),
                            source_is_capture: true,
                            manual: true,
                            assignee: meta.and_then(|m| m.assignee.clone()),
                            due: meta.and_then(|m| m.due.clone()),
                            note_id: Some(note_id.clone()),
                        });
                    }
                }
            }

            if let Some(org) = org {
                for task in org.tasks {
                    if task.line.is_some() {
                        continue;
                    }
                    entries.push(TaskEntryDto {
                        id: task.id,
                        path: path.clone(),
                        expected_line: String::new(),
                        text: task.text,
                        state: if task.done { "done" } else { "open" }.into(),
                        source_path: locator.clone(),
                        source_is_capture: true,
                        manual: false,
                        assignee: task.assignee,
                        due: task.due,
                        note_id: Some(note_id.clone()),
                    });
                }
            }
        }

        Ok((
            mapped_target.unwrap_or_default(),
            revision,
            entries,
            notices,
        ))
    }

    pub(crate) fn toggle_task(
        &mut self,
        target: &str,
        task_id: &str,
        expected_line: &str,
        done: bool,
    ) -> Result<bool, StorageError> {
        if let Some(rest) = task_id.strip_prefix("org:") {
            let (note_id, _) = rest
                .split_once(':')
                .ok_or_else(|| invalid("待办标识格式无效"))?;
            self.toggle_organized_task(note_id, task_id, done)?;
            return Ok(false);
        }

        let (physical, block_id) = target
            .split_once(NOTE_ANCHOR)
            .map_or((target, None), |(path, marker)| (path, Some(marker)));
        let (key, path) = self
            .workspace
            .files
            .markdown_file(physical)
            .map_err(file_error)?;
        wikra::ensure_editable(&self.connection, &key, "tasks")?;
        let before = utf8(fs::read(&path).map_err(file_error)?)?;
        let (mut anchored, manual, _) = parse_tasks(&before);
        let file_line = if let Some(number) = task_id.strip_prefix("line:") {
            let mut lines = task_lines(&before, block_id)
                .ok_or_else(|| conflict("任务所在记录已移动或删除，请刷新清单"))?;
            let file_line = number
                .parse::<usize>()
                .ok()
                .and_then(|line| line.checked_sub(1))
                .and_then(|index| lines.nth(index))
                .ok_or_else(|| conflict("任务行已移动或锚点变动，请刷新清单"))?;
            Some(file_line)
        } else {
            None
        };
        let task = if let Some(file_line) = file_line {
            manual
                .into_iter()
                .find(|(line, _)| *line == file_line)
                .map(|(_, task)| task)
        } else {
            anchored
                .remove(task_id)
                .and_then(|mut tasks| (tasks.len() == 1).then(|| tasks.pop().unwrap()))
        }
        .ok_or_else(|| conflict("任务行已移动或锚点变动，请刷新清单"))?;
        if task.line != expected_line || !matches!(task.state, "open" | "done") {
            return Err(conflict("任务行已被修改，请刷新清单后重新勾选"));
        }
        if (task.state == "done") == done {
            return Ok(false);
        }
        let mut after = before.clone();
        after.replace_range(
            task.checkbox..task.checkbox + 1,
            if done { "x" } else { " " },
        );
        replace_markdown(&path, Some(before.as_bytes()), after.as_bytes()).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                conflict("任务文件刚被修改，请刷新后重新勾选")
            } else {
                file_error(error)
            }
        })?;
        self.connection
            .execute(
                "UPDATE notes SET ov_resource_status = 'pending'
                 WHERE relative_path = ?1 AND ov_resource_status = 'synced'",
                [physical],
            )
            .map_err(db)?;
        if let Some(file_line) = file_line {
            self.sync_line_task_in_notes(physical, &before, file_line, &task.text, done);
        }
        Ok(true)
    }

    fn sync_line_task_in_notes(
        &mut self,
        physical_path: &str,
        markdown: &str,
        file_line: u32,
        text: &str,
        done: bool,
    ) {
        let Ok(mut stmt) = self.connection.prepare(
            "SELECT id, block_id, organize_json
             FROM notes WHERE relative_path = ?1 AND organize_json IS NOT NULL",
        ) else {
            return;
        };
        let Ok(rows) = stmt.query_map([physical_path], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        }) else {
            return;
        };
        for (note_id, block_id, raw) in rows.flatten() {
            let Some(lines) = task_lines(markdown, block_id.as_deref())
                .filter(|lines| lines.contains(&file_line))
            else {
                continue;
            };
            let note_line = file_line - lines.start + 1;
            let Ok(mut parsed) = serde_json::from_str::<crate::dto::OrganizeResultDto>(&raw) else {
                continue;
            };
            let mut changed = false;
            for item in &mut parsed.tasks {
                if item.line == Some(note_line) && item.text == text {
                    item.done = done;
                    changed = true;
                }
            }
            if changed && let Ok(updated) = serde_json::to_string(&parsed) {
                let _ = self.connection.execute(
                    "UPDATE notes SET organize_json = ?2 WHERE id = ?1",
                    params![note_id, updated],
                );
            }
        }
    }
}

struct ParsedTask {
    text: String,
    state: &'static str,
    checkbox: usize,
    line: String,
    line_number: usize,
    source_path: Option<String>,
}

fn task_lines(markdown: &str, block_id: Option<&str>) -> Option<Range<u32>> {
    let (start, body) = match block_id {
        Some(marker) => block_body_with_start(markdown, marker)?,
        None => (0, markdown),
    };
    let first = markdown[..start].lines().count() as u32 + 1;
    Some(first..first + body.lines().count() as u32)
}

fn parse_manual_task(content: &str) -> Option<(&str, &'static str, usize)> {
    let rest = if let Some(rest) = content.strip_prefix(['-', '*', '+']) {
        rest
    } else {
        let digits = content.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=9).contains(&digits) {
            return None;
        }
        content[digits..].strip_prefix(['.', ')'])?
    };
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim_start_matches([' ', '\t']);
    let state = match rest.as_bytes().get(..4) {
        Some(b"[ ] " | b"[ ]\t") => "open",
        Some(b"[x] " | b"[X] " | b"[x]\t" | b"[X]\t") => "done",
        Some(b"[-] " | b"[-]\t") => "cancelled",
        _ => return None,
    };
    let title = rest[4..].trim();
    (!title.is_empty()).then_some((title, state, content.len() - rest.len() + 1))
}

pub(super) fn document_tasks(body: &str) -> Vec<DocTaskItemDto> {
    let (anchored, manual, _) = parse_tasks(body);
    let mut tasks: Vec<_> = manual
        .into_iter()
        .map(|(line, task)| (format!("line:{line}"), task))
        .chain(anchored.into_iter().filter_map(|(id, mut tasks)| {
            (tasks.len() == 1).then(|| (id, tasks.pop().expect("one anchored task")))
        }))
        .filter(|(_, task)| matches!(task.state, "open" | "done"))
        .map(|(id, task)| DocTaskItemDto {
            id,
            line_number: task.line_number,
            expected_line: task.line,
            text: task.text,
            done: task.state == "done",
        })
        .collect();
    tasks.sort_unstable_by_key(|task| task.line_number);
    tasks
}

fn extract_source_path(tail: &str) -> Option<String> {
    let source = tail.split("（第 ").next()?.trim();
    (!source.is_empty()).then(|| source.to_owned())
}

fn parse_tasks(
    body: &str,
) -> (
    HashMap<String, Vec<ParsedTask>>,
    Vec<(u32, ParsedTask)>,
    Vec<String>,
) {
    let mut found: HashMap<String, Vec<ParsedTask>> = HashMap::new();
    let mut manual = Vec::new();
    let mut issues = Vec::new();
    let mut offset = 0;
    let mut fence: Option<(u8, usize)> = None;
    let mut in_comment = false;
    let mut frontmatter = false;
    for (number, raw) in body.split_inclusive('\n').enumerate() {
        let line_start = offset;
        offset += raw.len();
        let original = raw.trim_end_matches(['\r', '\n']);
        let line = if number == 0 {
            original.trim_start_matches('\u{feff}')
        } else {
            original
        };
        if number == 0 && line == "---" {
            frontmatter = true;
            continue;
        }
        if frontmatter {
            if matches!(line, "---" | "...") {
                frontmatter = false;
            }
            continue;
        }
        let content = line.trim_start_matches(' ');
        if in_comment {
            if content.contains("-->") {
                in_comment = false;
            }
            continue;
        }
        let marker = content.as_bytes().first().copied();
        if let Some(ch @ (b'`' | b'~')) = marker
            && line.len() - content.len() <= 3
        {
            let count = content.bytes().take_while(|&c| c == ch).count();
            match fence {
                None if count >= 3 => {
                    fence = Some((ch, count));
                    continue;
                }
                Some((previous, min))
                    if ch == previous && count >= min && content[count..].trim().is_empty() =>
                {
                    fence = None;
                    continue;
                }
                _ => (),
            }
        }
        if fence.is_some() {
            continue;
        }
        let mut remainder = content;
        while let Some((_, tail)) = remainder.split_once("<!--") {
            let Some((_, rest)) = tail.split_once("-->") else {
                in_comment = true;
                break;
            };
            remainder = rest;
        }
        if in_comment || content.starts_with("<!--") {
            continue;
        }
        let content_start = line_start + original.len() - content.len();
        let parsed = (line.len() - content.len() <= 3 && !content.starts_with('\t'))
            .then_some(content)
            .and_then(|content| {
                content
                    .split_once(" <!-- fmd-task:")
                    .and_then(|(head, tail)| {
                        let id = tail.strip_suffix(" -->")?;
                        (id.len() == 64
                            && id.bytes().all(|b| b.is_ascii_hexdigit())
                            && !head.contains("fmd-task:"))
                        .then_some((head, id))
                    })
            });
        let Some((head, id)) = parsed else {
            if content.contains("fmd-task:") {
                issues.push(format!("第 {} 行缺少有效的任务锚点", number + 1));
                for tail in content.split("fmd-task:").skip(1) {
                    let n = tail.bytes().take_while(|b| b.is_ascii_hexdigit()).count();
                    if n == 64 {
                        found.entry(tail[..n].into()).or_default().push(ParsedTask {
                            text: content.into(),
                            state: "invalid",
                            checkbox: 0,
                            line: original.to_owned(),
                            line_number: number + 1,
                            source_path: None,
                        });
                    }
                }
            } else if line.len() - content.len() <= 3
                && !content.starts_with('\t')
                && let Some((title, state, checkbox)) = parse_manual_task(content)
            {
                manual.push((
                    number as u32 + 1,
                    ParsedTask {
                        text: title.to_owned(),
                        state,
                        checkbox: content_start + checkbox,
                        line: original.to_owned(),
                        line_number: number + 1,
                        source_path: None,
                    },
                ));
            }
            continue;
        };
        let state = match head.as_bytes().get(..6) {
            Some(b"- [ ] ") => "open",
            Some(b"- [x] " | b"- [X] ") => "done",
            Some(b"- [-] ") => "cancelled",
            _ => "invalid",
        };
        let (text, source_path) = if state == "invalid" {
            issues.push(format!("第 {} 行的任务复选框损坏", number + 1));
            (head, None)
        } else {
            match head[6..].rsplit_once(" · 来源：") {
                Some((title, tail)) => (title, extract_source_path(tail)),
                None => (&head[6..], None),
            }
        };
        found.entry(id.into()).or_default().push(ParsedTask {
            text: text.into(),
            state,
            checkbox: content_start + 3,
            line: original.to_owned(),
            line_number: number + 1,
            source_path,
        });
    }
    (found, manual, issues)
}

#[cfg(test)]
mod tests {
    use super::super::tests::TestDir;
    use super::*;
    use std::{cell::RefCell, fs, process::Command};

    thread_local! {
        static BEFORE_WRITE: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    }
    pub(super) fn at_write_boundary() {
        BEFORE_WRITE.with_borrow_mut(|hook| {
            if let Some(run) = hook.take() {
                run();
            }
        });
    }

    #[test]
    fn lists_wiki_documents_without_captures_or_task_list() {
        for capture_dir in ["notes", "inbox/daily"] {
            let dir = TestDir::new();
            let mut storage = Storage::open(&dir.0, Path::new(capture_dir), ".flash-test").unwrap();
            fs::write(dir.0.join(capture_dir).join("untracked.md"), "每日原文").unwrap();
            fs::write(dir.0.join("legacy.md"), "历史随手记").unwrap();
            let legacy = storage.ensure_note_for_source("legacy.md").unwrap();
            storage
                .connection
                .execute(
                    "UPDATE notes SET draft_id = 'legacy-capture' WHERE id = ?1",
                    [&legacy],
                )
                .unwrap();
            fs::write(dir.0.join("notes-index.md"), "# 首页\n\n项目资料").unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(dir.0.join("notes-index.md"))
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
                .unwrap();
            fs::write(dir.0.join("bad.md"), [0xff, 0xfe]).unwrap();
            fs::write(dir.0.join(".flash-test/hidden.md"), "内部状态").unwrap();

            let target = "wiki/architecture.md";
            storage.create_wiki_target(target).unwrap();
            fs::write(dir.0.join("wiki/topics.md"), "# 主题").unwrap();
            fs::write(dir.0.join("wiki/tasks.md"), "- [ ] 待办").unwrap();
            storage.map_wiki_target("tasks", "wiki/tasks.md").unwrap();

            let applied = storage
                .create_wiki_suggestion(None, target, "已确认的决定", "采纳理由")
                .unwrap();
            storage.apply_wiki_suggestion(&applied.id).unwrap();
            let conflict = storage
                .create_wiki_suggestion(None, target, "待核对的内容", "冲突理由")
                .unwrap();
            fs::write(dir.0.join(target), "# 架构设计\n\n手工补充").unwrap();
            assert_eq!(
                storage
                    .apply_wiki_suggestion(&conflict.id)
                    .unwrap_err()
                    .code,
                "VERSION_CONFLICT"
            );
            let pending = storage
                .create_wiki_suggestion(None, target, "候选设计", "待审理由")
                .unwrap();

            let docs = storage.list_wiki_documents().unwrap();
            assert_eq!(
                docs.iter().map(|doc| doc.path.as_str()).collect::<Vec<_>>(),
                [target, "notes-index.md", "wiki/topics.md"]
            );
            let mapped = &docs[0];
            assert!(mapped.mapped);
            assert_eq!(
                (&mapped.title[..], &mapped.folder[..]),
                ("架构设计", "wiki")
            );
            assert_eq!(
                (mapped.pending_suggestions, mapped.conflict_suggestions),
                (1, 1)
            );
            assert!(mapped.preview.contains("手工补充"));
            let root = &docs[1];
            assert!(!root.mapped && root.folder.is_empty());
            assert_eq!(root.updated_at.as_deref(), Some("2023-11-14T22:13:20Z"));
            let detail = storage.get_note_detail(target).unwrap();
            assert_eq!(
                detail
                    .wiki_suggestions
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                [pending.id, conflict.id, applied.id]
            );
        }
    }

    #[test]
    fn creates_opens_and_indexes_nested_wiki_pages() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        let mut search = storage.search_reader().unwrap();
        search.rebuild_search_index().unwrap();

        let path = "wiki/arch/overview.md";
        storage.create_wiki_target(path).unwrap();
        assert!(fs::read(dir.0.join(path)).unwrap().is_empty());
        let blank = storage.list_wiki_documents().unwrap().pop().unwrap();
        assert_eq!(
            (&blank.title[..], &blank.folder[..]),
            ("overview", "wiki/arch")
        );
        assert!(blank.mapped && blank.preview.is_empty());
        // A disk-backed page stays readable even before its index entry exists.
        assert_eq!(
            storage
                .connection
                .execute(
                    "DELETE FROM search_documents WHERE relative_path = ?1",
                    [path]
                )
                .unwrap(),
            1
        );
        let detail = storage.get_note_detail(r"wiki\arch\overview.md").unwrap();
        assert_eq!(detail.id, format!("file:{path}"));
        assert_eq!(detail.path, path);
        assert!(!detail.is_capture && detail.body.is_empty() && detail.captured_at.is_none());
        assert_eq!(
            storage
                .get_note_detail(&format!("{path}{NOTE_ANCHOR}missing"))
                .unwrap_err()
                .code,
            "SOURCE_UNAVAILABLE"
        );

        storage
            .update_note(path, &detail.content_hash, "# 架构总览\n\n核心原则")
            .unwrap();
        assert!(storage.list_notes().unwrap().is_empty());
        let (hits, _, _) = search.search_records("核心原则", None, None, 0).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, path);
        assert_eq!(
            storage.get_note_detail(path).unwrap().body,
            "# 架构总览\n\n核心原则\n"
        );
    }

    #[test]
    fn manually_organized_documents_remain_wiki_candidates() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        let path = "wiki.md";
        fs::write(dir.0.join(path), "# 项目 Wiki").unwrap();
        let id = storage.ensure_note_for_source(path).unwrap();

        assert!(!storage.get_note_detail(&id).unwrap().is_capture);
        assert!(!storage.is_capture_path(path));
        assert!(storage.list_notes().unwrap().is_empty());
        assert!(storage.list_recent_captures().unwrap().is_empty());
        assert!(!storage.capture_paths().unwrap().contains(path));
        let docs = storage.list_wiki_documents().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].path, path);
        assert!(!docs[0].mapped);
        storage.map_wiki_target("wiki", path).unwrap();
        assert!(storage.list_wiki_documents().unwrap()[0].mapped);
    }

    #[test]
    fn capture_subdirectory_cannot_be_used_for_wiki_or_task_targets() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        fs::create_dir(dir.0.join("notes/sub")).unwrap();
        let existing = "notes/sub/capture.md";
        fs::write(dir.0.join(existing), "# 随手记").unwrap();
        for kind in ["wiki", "tasks"] {
            let error = storage.map_wiki_target(kind, existing).unwrap_err();
            assert_eq!(
                (error.code, error.safe_message),
                ("INVALID_INPUT", CAPTURE_TARGET_MESSAGE)
            );
        }
        let new_path = "notes/sub/new-wiki.md";
        assert_eq!(
            storage.create_wiki_target(new_path).unwrap_err().code,
            "INVALID_INPUT"
        );
        assert!(!dir.0.join(new_path).exists());
        assert!(storage.wiki_targets().unwrap().is_empty());
    }

    #[test]
    fn wiki_suggestions_apply_recompare_reject_undo_and_unmap_preserve_files() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        for path in ["a.md", "b.md"] {
            storage.create_wiki_target(path).unwrap();
        }
        fs::write(dir.0.join("a.md"), "\u{feff}人工原稿").unwrap();
        assert!(storage.create_wiki_target("a.md").is_err());
        assert!(storage.create_wiki_target(".flash-test/hidden.md").is_err());
        assert!(storage.create_wiki_target("../outside.md").is_err());

        let a = storage
            .create_wiki_suggestion(None, "a.md", "采用的意见", "同步决策")
            .unwrap();
        let b = storage
            .create_wiki_suggestion(None, "b.md", "尚待讨论", "补充说明")
            .unwrap();
        let mut search = storage.search_reader().unwrap();
        search.rebuild_search_index().unwrap();

        fs::write(dir.0.join("b.md"), "其他编辑器的意见").unwrap();
        assert_eq!(storage.apply_wiki_suggestion(&a.id).unwrap().1, "applied");
        assert_eq!(
            search
                .search_records("采用的意见", None, None, 0)
                .unwrap()
                .0[0]
                .0,
            "a.md"
        );
        assert_eq!(
            storage.apply_wiki_suggestion(&b.id).unwrap_err().code,
            "VERSION_CONFLICT"
        );
        assert_eq!(
            fs::read_to_string(dir.0.join("b.md")).unwrap(),
            "其他编辑器的意见"
        );

        let revised = storage.recompare_wiki_suggestion(&b.id).unwrap();
        assert_eq!(revised.status, "pending");
        assert!(
            revised.after_text.contains("其他编辑器的意见")
                && revised.after_text.contains("尚待讨论")
        );
        storage.reject_wiki_suggestion(&revised.id).unwrap();
        assert_eq!(
            storage
                .list_wiki_suggestions()
                .into_iter()
                .next()
                .unwrap()
                .iter()
                .find(|s| s.id == revised.id)
                .unwrap()
                .status,
            "rejected"
        );

        // Undo preserves subsequent human edits appended after the applied block.
        let applied_a = fs::read_to_string(dir.0.join("a.md")).unwrap();
        fs::write(dir.0.join("a.md"), format!("{applied_a}另一项手工补充\n")).unwrap();
        let reverted = storage.undo_wiki_suggestion(&a.id).unwrap();
        assert_eq!(reverted.status, "reverted");
        assert!(
            search
                .search_records("采用的意见", None, None, 0)
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(
            fs::read_to_string(dir.0.join("a.md")).unwrap(),
            "\u{feff}人工原稿\n另一项手工补充\n"
        );

        // Unmapping removes the wiki target without touching the file on disk.
        storage.unmap_wiki_target("b.md").unwrap();
        assert_eq!(
            storage.wiki_targets().unwrap(),
            [("wiki".into(), "a.md".into())]
        );
        assert!(dir.0.join("b.md").is_file());
    }

    #[test]
    fn toggles_only_the_selected_task_in_a_daily_capture_block() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        storage
            .set_model_config("http://127.0.0.1:1".into(), "fixture".into(), true)
            .unwrap();
        let mut records = Vec::new();
        for (revision, assignee) in [(1, "甲"), (2, "乙")] {
            let body = format!("负责人：{assignee}\r\n- [ ] 核对接口");
            storage.save_draft("capture", revision, &body).unwrap();
            let saved = storage
                .save_record(
                    &format!("1795579200000-{revision}"),
                    "capture",
                    revision,
                    &body,
                )
                .unwrap();
            storage
                .enqueue_capture_jobs(&saved.record_id, true, None)
                .unwrap();
            let job = storage.next_background_job(None).unwrap().unwrap();
            let input = storage.prepare_organize_job(&job).unwrap();
            storage
                .complete_organize_job(
                    &input,
                    Ok(serde_json::json!({
                        "tasks": [{"line": 2, "text": "核对接口", "assignee": assignee}]
                    })),
                )
                .unwrap();
            records.push(storage.get_note_detail(&saved.record_id).unwrap());
        }
        let [first, second] = records.as_slice() else {
            panic!("expected two capture blocks");
        };
        assert_eq!(first.path, second.path);
        let task = &second.organize.as_ref().unwrap().tasks[0];
        let line = task.line.unwrap();
        assert_eq!(line, 2);
        storage
            .toggle_task(
                &second.locator,
                &task.id,
                second.body.lines().nth(line as usize - 1).unwrap(),
                true,
            )
            .unwrap();

        let first_after = storage.get_note_detail(&first.id).unwrap();
        assert_eq!(first_after.body, first.body);
        assert!(!first_after.organize.unwrap().tasks[0].done);
        let second_after = storage.get_note_detail(&second.id).unwrap();
        assert_eq!(second_after.body, "负责人：乙\r\n- [x] 核对接口");
        assert!(second_after.organize.unwrap().tasks[0].done);

        let (_, _, tasks, _) = storage.list_all_tasks().unwrap();
        assert_eq!(tasks.len(), 2);
        let first_task = tasks.iter().find(|t| t.path == first.locator).unwrap();
        assert_eq!(first_task.assignee.as_deref(), Some("甲"));
        assert_eq!(first_task.state, "open");
        let second_task = tasks.iter().find(|t| t.path == second.locator).unwrap();
        assert_eq!(second_task.assignee.as_deref(), Some("乙"));
        assert_eq!(second_task.state, "done");
        storage
            .toggle_task(
                &second_task.path,
                &second_task.id,
                &second_task.expected_line,
                false,
            )
            .unwrap();
        let unchecked = storage.get_note_detail(&second.id).unwrap();
        assert_eq!(unchecked.body, second.body);
        assert!(!unchecked.organize.unwrap().tasks[0].done);
    }

    #[test]
    fn manual_tasks_and_links_coexist_with_anchored_tasks_without_polluting_wiki() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        let path = dir.0.join("Tasks.md");
        let task_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        fs::write(
            &path,
            format!(
                "\u{feff}# 待办\r\n- [ ] 应用内采用任务 · 来源：daily.md（第 2–2 行） <!-- fmd-task:{task_id} -->\r\n  * [ ] 手工待办\r\n- [X] 已完成手工任务\r\n- [参考文档](ref.md)\r\n"
            ),
        )
        .unwrap();
        storage.map_wiki_target("tasks", "Tasks.md").unwrap();
        let (target, _, tasks, notices) = storage.list_all_tasks().unwrap();
        assert!(notices.is_empty());
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].text, "应用内采用任务");
        assert_eq!(tasks[0].source_path, "daily.md");
        assert_eq!(
            (
                tasks[1].id.as_str(),
                tasks[1].text.as_str(),
                tasks[1].state.as_str()
            ),
            ("line:3", "手工待办", "open")
        );
        assert_eq!(
            (
                tasks[2].id.as_str(),
                tasks[2].text.as_str(),
                tasks[2].state.as_str()
            ),
            ("line:4", "已完成手工任务", "done")
        );

        storage
            .toggle_task(&target, &tasks[1].id, &tasks[1].expected_line, true)
            .unwrap();
        storage
            .toggle_task(&target, &tasks[2].id, &tasks[2].expected_line, false)
            .unwrap();
        let updated = fs::read_to_string(&path).unwrap();
        assert!(updated.contains("  * [x] 手工待办") && updated.contains("- [ ] 已完成手工任务"));
        assert!(storage.list_wiki_suggestions().unwrap().is_empty());

        let (_, _, refreshed, _) = storage.list_all_tasks().unwrap();
        let edited = updated.replace("手工待办", "手工已改");
        fs::write(&path, &edited).unwrap();
        assert_eq!(
            storage
                .toggle_task(
                    &target,
                    &refreshed[1].id,
                    &refreshed[1].expected_line,
                    false
                )
                .unwrap_err()
                .code,
            "VERSION_CONFLICT"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
    }

    #[test]
    #[ignore = "requires a real Vim executable in FMD_TEST_VIM"]
    fn real_vim_write_at_the_checked_file_boundary_preserves_human_edits() {
        let vim = std::env::var_os("FMD_TEST_VIM").expect("FMD_TEST_VIM is required");
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        storage.create_wiki_target("editor.md").unwrap();
        let suggestion = storage
            .create_wiki_suggestion(None, "editor.md", "应用改动", "测试并发")
            .unwrap();
        let root = dir.0.clone();
        BEFORE_WRITE.with_borrow_mut(|hook| {
            *hook = Some(Box::new(move || {
                let output = Command::new(vim)
                    .current_dir(root)
                    .args([
                        "-Nu",
                        "NONE",
                        "-i",
                        "NONE",
                        "-n",
                        "-es",
                        "--noplugin",
                        "editor.md",
                        "-c",
                        "call setline(1, 'human-edit')",
                        "-c",
                        "wq",
                    ])
                    .output()
                    .unwrap();
                assert!(output.status.success());
            }));
        });
        assert_eq!(
            storage
                .apply_wiki_suggestion(&suggestion.id)
                .unwrap_err()
                .code,
            "VERSION_CONFLICT"
        );
        let path = dir.0.join("editor.md");
        assert_eq!(fs::read_to_string(&path).unwrap().trim_end(), "human-edit");
        let recompared = storage.recompare_wiki_suggestion(&suggestion.id).unwrap();
        storage.apply_wiki_suggestion(&recompared.id).unwrap();
        let result = fs::read_to_string(path).unwrap();
        assert!(result.starts_with("human-edit") && result.contains("应用改动"));
    }
}
