use std::path::PathBuf;

use rusqlite::Transaction;

use super::*;

pub(crate) mod agent;
pub(crate) mod documents;
pub(crate) mod inbox;
pub(crate) mod lint;
pub(crate) mod markdown;
pub(crate) mod organize;

#[cfg(test)]
mod p0_tests;
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

pub(super) const MIGRATE_V8: &str = "
ALTER TABLE wikra_changesets ADD COLUMN parent_id TEXT REFERENCES wikra_changesets(id);
ALTER TABLE wikra_changesets ADD COLUMN note TEXT NOT NULL DEFAULT '';
ALTER TABLE wikra_changesets ADD COLUMN dropped_claims INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS wikra_changeset_pages (
    changeset_id TEXT NOT NULL REFERENCES wikra_changesets(id) ON DELETE CASCADE,
    path TEXT NOT NULL, title TEXT NOT NULL, before_snapshot TEXT, after_hash TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('applied', 'reverted', 'kept_modified')),
    PRIMARY KEY (changeset_id, path)
);
CREATE TABLE IF NOT EXISTS wikra_changeset_ops (
    changeset_id TEXT NOT NULL REFERENCES wikra_changesets(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL, op TEXT NOT NULL, page_path TEXT NOT NULL, page_title TEXT NOT NULL,
    label TEXT NOT NULL, payload_json TEXT NOT NULL, base_hash TEXT,
    outcome TEXT NOT NULL CHECK(outcome IN ('applied', 'pending_review', 'discarded', 'dropped')),
    reason TEXT NOT NULL DEFAULT '', conflict_id TEXT REFERENCES conflicts(id),
    child_changeset_id TEXT REFERENCES wikra_changesets(id),
    PRIMARY KEY (changeset_id, seq)
);
CREATE INDEX IF NOT EXISTS wikra_changeset_ops_pending ON wikra_changeset_ops(outcome, page_path);
CREATE TABLE IF NOT EXISTS wikra_lint_findings (
    id TEXT PRIMARY KEY, rule TEXT NOT NULL, paths_json TEXT NOT NULL, page_hashes_json TEXT NOT NULL,
    detail_json TEXT NOT NULL, created_at TEXT NOT NULL
);

INSERT OR IGNORE INTO wikra_changeset_pages (changeset_id, path, title, before_snapshot, after_hash, state)
SELECT id, target_path, target_title, before_snapshot, after_hash,
       CASE status WHEN 'reverted' THEN 'reverted' ELSE 'applied' END
FROM wikra_changesets;
INSERT OR IGNORE INTO wikra_changeset_ops (changeset_id, seq, op, page_path, page_title, label, payload_json, outcome)
SELECT id, 1, CASE WHEN before_snapshot IS NULL THEN 'create_page' ELSE 'append_section' END,
       target_path, target_title, op_label, '{}',
       CASE status WHEN 'reverted' THEN 'discarded' ELSE 'applied' END
FROM wikra_changesets;
UPDATE wikra_changesets SET before_snapshot = NULL;";

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
        let key = self.key.clone();
        MarkdownBatch(vec![(self, after.map(str::to_owned))]).commit(tx, stage)?;
        Ok(key)
    }
}

#[derive(Default)]
struct MarkdownBatch(Vec<(MarkdownEdit, Option<String>)>);

impl MarkdownBatch {
    fn push(&mut self, edit: MarkdownEdit, after: Option<String>) {
        self.0.push((edit, after));
    }

    fn commit(self, tx: Transaction<'_>, stage: &'static str) -> Result<(), StorageError> {
        let db = |err| database_error(stage, None, err);
        for (edit, after) in &self.0 {
            match after {
                Some(body) => wiki::update_wiki_index(&tx, &edit.key, body)?,
                None => {
                    tx.execute(
                        "DELETE FROM search_documents WHERE relative_path = ?1",
                        [&edit.key],
                    )
                    .map_err(db)?;
                }
            }
        }
        for (index, (edit, after)) in self.0.iter().enumerate() {
            if let Err(err) = write_edit(&edit.path, edit.before.as_deref(), after.as_deref()) {
                let failure = if err.kind() == io::ErrorKind::AlreadyExists
                    || err.kind() == io::ErrorKind::NotFound && edit.before.is_some()
                {
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
                };
                return Err(self.restore(index, stage, failure));
            }
        }
        tx.commit()
            .map_err(|err| self.restore(self.0.len(), stage, db(err)))
    }

    fn restore(&self, written: usize, stage: &'static str, failure: StorageError) -> StorageError {
        let mut errors = Vec::new();
        for (edit, after) in self.0[..written].iter().rev() {
            if let Err(err) = write_edit(&edit.path, after.as_deref(), edit.before.as_deref()) {
                errors.push(format!("{}: {err}", edit.key));
            }
        }
        if errors.is_empty() {
            failure
        } else {
            error(
                "WRITE_FAILED",
                stage,
                None,
                true,
                "部分文件未能恢复，请检查当前 Markdown",
                format!("{}; {}", failure.technical_detail, errors.join("; ")),
            )
        }
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
