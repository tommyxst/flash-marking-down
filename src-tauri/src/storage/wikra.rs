use std::path::PathBuf;

use rusqlite::Transaction;

use super::*;

mod documents;
pub(crate) mod inbox;
mod markdown;
pub(crate) mod organize;

#[cfg(test)]
mod tests;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS inbox_items (
    id TEXT PRIMARY KEY, title TEXT NOT NULL, domain TEXT NOT NULL, source_type TEXT NOT NULL,
    url TEXT, raw_path TEXT NOT NULL UNIQUE, raw_hash TEXT NOT NULL, saved_at TEXT NOT NULL,
    fetch_status TEXT NOT NULL CHECK(fetch_status IN ('full', 'stub')),
    state TEXT NOT NULL CHECK(state IN ('unread', 'reading', 'compiled', 'archived')),
    word_count INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS inbox_paragraphs (
    item_id TEXT NOT NULL REFERENCES inbox_items(id) ON DELETE CASCADE,
    position INTEGER NOT NULL, heading TEXT NOT NULL, text TEXT NOT NULL,
    highlighted INTEGER NOT NULL DEFAULT 0, compiled INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (item_id, position)
);
CREATE TABLE IF NOT EXISTS wikra_changesets (
    id TEXT PRIMARY KEY, created_at TEXT NOT NULL, trigger TEXT NOT NULL, model_alias TEXT NOT NULL,
    status TEXT NOT NULL, summary TEXT NOT NULL,
    target_path TEXT NOT NULL, target_title TEXT NOT NULL, op_label TEXT NOT NULL,
    before_snapshot TEXT, after_hash TEXT NOT NULL, inbox_id TEXT REFERENCES inbox_items(id),
    paragraphs_json TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS wikra_changesets_inbox ON wikra_changesets(inbox_id, status);
CREATE TABLE IF NOT EXISTS capture_buffer_queue (
    note_id TEXT PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
    queued_at TEXT NOT NULL, due_at INTEGER
);
CREATE TABLE IF NOT EXISTS dismissed_suggestions (id TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS dismissed_graph_gaps (id TEXT PRIMARY KEY);";

fn invalid(stage: &'static str, message: &'static str) -> StorageError {
    error("INVALID_INPUT", stage, None, false, message, message)
}

fn conflict(stage: &'static str, message: &'static str) -> StorageError {
    error("VERSION_CONFLICT", stage, None, true, message, message)
}

fn source_error(stage: &'static str, detail: impl std::fmt::Display) -> StorageError {
    error(
        "SOURCE_UNAVAILABLE",
        stage,
        None,
        true,
        "无法读取 Markdown，请检查路径、权限或文件编码",
        detail,
    )
}

fn database_identity(
    conn: &Connection,
    stage: &'static str,
) -> Result<(String, String), StorageError> {
    conn.query_row(
        "SELECT lower(hex(randomblob(16))), strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(|err| database_error(stage, None, err))
}

pub(super) fn ensure_editable(
    conn: &Connection,
    path: &str,
    stage: &'static str,
) -> Result<(), StorageError> {
    let immutable: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM inbox_items WHERE raw_path = ?1)",
            [path],
            |row| row.get(0),
        )
        .map_err(|err| database_error(stage, None, err))?;
    if immutable {
        return Err(invalid(stage, "收藏原文仅可在收件箱补全缺失正文"));
    }
    Ok(())
}

struct MarkdownEdit {
    key: String,
    path: PathBuf,
    before: Option<String>,
}

impl MarkdownEdit {
    fn load(
        files: &WorkspaceFiles,
        relative_path: &str,
        allow_create: bool,
        stage: &'static str,
    ) -> Result<Self, StorageError> {
        let target = if allow_create {
            files.create_markdown_target(relative_path)
        } else {
            files.markdown_file(relative_path)
        };
        let read_error = |err: io::Error| {
            if err.kind() == io::ErrorKind::NotFound && !allow_create {
                conflict(stage, "Markdown 文件已移动或删除，请刷新后重试")
            } else {
                source_error(stage, err)
            }
        };
        let (key, path) = target.map_err(read_error)?;
        let before = match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == io::ErrorKind::NotFound && allow_create => None,
            Err(err) => return Err(read_error(err)),
        };
        Ok(Self { key, path, before })
    }

    fn commit(
        self,
        tx: Transaction<'_>,
        after: Option<&str>,
        stage: &'static str,
    ) -> Result<String, StorageError> {
        let db = |err| database_error(stage, None, err);
        match after {
            Some(body) => wiki::update_wiki_index(&tx, &self.key, body)?,
            None => {
                tx.execute(
                    "DELETE FROM search_documents WHERE relative_path = ?1",
                    [&self.key],
                )
                .map_err(db)?;
            }
        }
        let before = self.before.as_deref();
        write_edit(&self.path, before, after).map_err(|err| {
            if err.kind() == io::ErrorKind::AlreadyExists {
                conflict(stage, "Markdown 已在外部被修改，请刷新后重试")
            } else {
                error(
                    "WRITE_FAILED",
                    stage,
                    None,
                    true,
                    "无法写入 Markdown 文件",
                    err,
                )
            }
        })?;
        if let Err(err) = tx.commit() {
            // The file is already visible. Restore only if it still matches our write.
            if let Err(restore) = write_edit(&self.path, after, before) {
                return Err(error(
                    "WRITE_FAILED",
                    stage,
                    None,
                    true,
                    "状态库保存失败，请刷新并检查 Markdown 文件",
                    format!("{err}; restoring Markdown: {restore}"),
                ));
            }
            return Err(db(err));
        }
        Ok(self.key)
    }
}

fn write_edit(path: &Path, before: Option<&str>, after: Option<&str>) -> io::Result<()> {
    if let Some(body) = after {
        return replace_markdown(path, before.map(str::as_bytes), body.as_bytes());
    }
    match fs::read_to_string(path) {
        Ok(text) if before == Some(text.as_str()) => fs::remove_file(path),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Markdown changed on disk",
        )),
        Err(err) => Err(err),
    }
}
