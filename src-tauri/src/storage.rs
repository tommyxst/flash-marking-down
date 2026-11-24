use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::workspace::WorkspaceFiles;

mod search;
pub(crate) use search::SearchReader;
pub(crate) mod m2;
pub(crate) mod m2_answer;
pub(crate) mod wiki;

const RECENT_CAPTURE_LIMIT: i64 = 50;

pub(crate) type RecentCapture = (String, String, String, String);

#[derive(Debug)]
pub struct StorageError {
    pub code: &'static str,
    pub stage: &'static str,
    pub operation_id: Option<String>,
    pub retryable: bool,
    pub safe_message: &'static str,
    pub technical_detail: String,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.safe_message)
    }
}

impl std::error::Error for StorageError {}

pub(crate) fn error(
    code: &'static str,
    stage: &'static str,
    operation_id: Option<&str>,
    retryable: bool,
    safe_message: &'static str,
    detail: impl std::fmt::Display,
) -> StorageError {
    StorageError {
        code,
        stage,
        operation_id: operation_id.map(str::to_owned),
        retryable,
        safe_message,
        technical_detail: detail.to_string(),
    }
}

fn database_error(
    stage: &'static str,
    operation_id: Option<&str>,
    detail: rusqlite::Error,
) -> StorageError {
    error(
        "WRITE_FAILED",
        stage,
        operation_id,
        true,
        "本地状态库操作失败",
        detail,
    )
}

#[derive(Debug, PartialEq, Eq)]
pub struct Draft {
    pub id: String,
    pub body: String,
    pub edit_revision: i64,
    pub persisted_revision: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedRecord {
    pub record_id: String,
    pub source_id: String,
    pub source_revision: String,
    pub relative_path: String,
    pub saved_at: String,
}

#[derive(Debug)]
pub struct RecoveryOutcome {
    pub request_id: String,
    pub result: Result<SavedRecord, StorageError>,
}

struct Intent {
    draft_id: String,
    draft_revision: i64,
    body_hash: String,
    record_id: String,
    source_id: String,
    #[cfg_attr(not(test), allow(dead_code))]
    file_name: String,
    relative_path: String,
    bytes: Option<Vec<u8>>,
    revision: String,
    captured_at: String,
    state: String,
    saved_at: Option<String>,
}

pub struct Storage {
    connection: Connection,
    files: Arc<WorkspaceFiles>,
    search_build: Arc<Mutex<()>>,
    capture_dir: String,
    workspace_id: String,
}

impl Storage {
    pub fn open(
        root: &Path,
        capture_dir: &Path,
        state_dir_name: &str,
    ) -> Result<Self, StorageError> {
        Self::open_inner(root, capture_dir, state_dir_name, None)
    }

    pub(crate) fn reopen(
        root: &Path,
        capture_dir: &Path,
        state_dir_name: &str,
        workspace_id: &str,
        file_identity: &str,
    ) -> Result<Self, StorageError> {
        Self::open_inner(
            root,
            capture_dir,
            state_dir_name,
            Some((workspace_id, file_identity)),
        )
    }

    fn open_inner(
        root: &Path,
        capture_dir: &Path,
        state_dir_name: &str,
        expected: Option<(&str, &str)>,
    ) -> Result<Self, StorageError> {
        let relative_capture = capture_directory(capture_dir)?;
        let files =
            WorkspaceFiles::open(root, capture_dir, state_dir_name, expected.map(|ids| ids.1))
                .map_err(|e| error("WRITE_FAILED", "workspace", None, true, "无法打开工作区", e))?;
        let connection = Connection::open_with_flags(
            files.database_path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| database_error("open", None, e))?;
        connection
            .execute_batch(
                "PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;",
            )
            .map_err(|e| database_error("open", None, e))?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|e| database_error("migration", None, e))?;
        let application_id: i64 = connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(|e| database_error("migration", None, e))?;
        match version {
            0 if application_id == 0 && expected.is_none() => {
                let count: i64 = connection
                    .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
                    .map_err(|e| database_error("migration", None, e))?;
                if count != 0 {
                    return Err(error(
                        "CAPABILITY_UNSUPPORTED",
                        "migration",
                        None,
                        false,
                        "状态库不是空库",
                        "unrecognized SQLite schema",
                    ));
                }
                connection
                    .execute_batch(SCHEMA)
                    .map_err(|e| database_error("migration", None, e))?;
                connection
                    .execute_batch(search::SCHEMA)
                    .map_err(|e| database_error("migration", None, e))?;
                connection
                    .execute_batch(m2::SCHEMA)
                    .and_then(|_| connection.execute_batch(m2_answer::SCHEMA))
                    .and_then(|_| connection.execute_batch(wiki::SCHEMA))
                    .map_err(|e| database_error("migration", None, e))?;
                connection
                    .execute(
                        "INSERT INTO workspace_settings (key, value)
                         VALUES ('capture_dir', ?1), ('id', lower(hex(randomblob(16))))",
                        [&relative_capture],
                    )
                    .map_err(|e| database_error("migration", None, e))?;
                connection
                    .pragma_update(None, "application_id", APPLICATION_ID)
                    .map_err(|e| database_error("migration", None, e))?;
                connection
                    .pragma_update(None, "user_version", SCHEMA_VERSION)
                    .map_err(|e| database_error("migration", None, e))?;
                connection
                    .execute_batch("COMMIT;")
                    .map_err(|e| database_error("migration", None, e))?;
            }
            1..=SCHEMA_VERSION if application_id == APPLICATION_ID => (),
            _ => {
                return Err(error(
                    "CAPABILITY_UNSUPPORTED",
                    "migration",
                    None,
                    false,
                    "工作区状态库版本不受支持",
                    version,
                ));
            }
        }
        let persisted_dir: String = connection
            .query_row(
                "SELECT value FROM workspace_settings WHERE key = 'capture_dir'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| database_error("open", None, e))?;
        if expected.is_some() {
            verify_capture_directory(&relative_capture, &persisted_dir)?;
        }
        let workspace_id = connection
            .query_row(
                "SELECT value FROM workspace_settings WHERE key = 'id'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| database_error("open", None, e))?;
        let mut storage = Self {
            connection,
            files: Arc::new(files),
            search_build: Arc::default(),
            capture_dir: relative_capture,
            workspace_id,
        };
        if let Some((workspace_id, file_identity)) = expected {
            storage.verify_identity(workspace_id, file_identity)?;
        }
        storage
            .connection
            .execute_batch("PRAGMA journal_mode = WAL;")
            .map_err(|e| database_error("open", None, e))?;
        if expected.is_none() && storage.capture_dir != persisted_dir {
            storage
                .connection
                .execute(
                    "UPDATE workspace_settings SET value = ?1 WHERE key = 'capture_dir'",
                    [&storage.capture_dir],
                )
                .map_err(|e| database_error("open", None, e))?;
        }
        if version > 0 && version < SCHEMA_VERSION {
            let tx = storage
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|e| database_error("migration", None, e))?;
            if version == 1 {
                tx.execute_batch(search::SCHEMA)
                    .map_err(|e| database_error("migration", None, e))?;
            } else if version < 4 {
                tx.execute_batch(search::MIGRATE_V4)
                    .map_err(|e| database_error("migration", None, e))?;
            }
            tx.execute_batch(m2::SCHEMA)
                .and_then(|_| tx.execute_batch(m2_answer::SCHEMA))
                .and_then(|_| tx.execute_batch(wiki::SCHEMA))
                .map_err(|e| database_error("migration", None, e))?;
            if version < 4 {
                m2_answer::migrate_legacy_disagreements(&tx).map_err(|detail| {
                    error(
                        "WRITE_FAILED",
                        "migration",
                        None,
                        false,
                        "迁移分歧数据失败",
                        detail,
                    )
                })?;
            }
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(|e| database_error("migration", None, e))?;
            tx.commit()
                .map_err(|e| database_error("migration", None, e))?;
        }
        storage.connection.execute(
            "UPDATE jobs SET state='failed' WHERE state='running' AND id IN (SELECT id FROM m2_analyses)", [],
        ).map_err(|e| database_error("recover_analysis", None, e))?;
        Ok(storage)
    }

    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    pub(crate) fn file_identity(&self) -> &str {
        self.files.file_identity()
    }

    pub(crate) fn matches_root(&self, root: &Path) -> Result<bool, StorageError> {
        self.files
            .matches_root(root)
            .map_err(|e| error("WRITE_FAILED", "workspace", None, true, "无法打开工作区", e))
    }

    pub(crate) fn capture_dir(&self) -> &str {
        &self.capture_dir
    }

    pub(crate) fn verify_capture_dir(&self, requested: &Path) -> Result<(), StorageError> {
        verify_capture_directory(&capture_directory(requested)?, &self.capture_dir)
    }

    pub(crate) fn update_capture_dir(&mut self, requested: &Path) -> Result<(), StorageError> {
        let relative = capture_directory(requested)?;
        if relative == self.capture_dir {
            return Ok(());
        }
        self.files.update_capture_dir(requested).map_err(|e| {
            error(
                "WRITE_FAILED",
                "workspace",
                None,
                true,
                "无法使用该保存目录",
                e,
            )
        })?;
        self.connection
            .execute(
                "UPDATE workspace_settings SET value = ?1 WHERE key = 'capture_dir'",
                [&relative],
            )
            .map_err(|e| database_error("workspace", None, e))?;
        self.capture_dir = relative;
        Ok(())
    }

    pub(crate) fn workspace_files(&self) -> Arc<WorkspaceFiles> {
        self.files.clone()
    }

    pub(crate) fn verify_identity(
        &self,
        workspace_id: &str,
        file_identity: &str,
    ) -> Result<(), StorageError> {
        if self.workspace_id() != workspace_id || self.file_identity() != file_identity {
            return Err(error(
                "VERSION_CONFLICT",
                "workspace",
                None,
                false,
                "工作区已变更，请重新选择",
                "workspace identity differs from the saved selection",
            ));
        }
        Ok(())
    }

    pub(crate) fn credential_reference(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.connection
            .query_row(
                "SELECT value FROM workspace_settings WHERE key = ?1",
                [format!("credential:{key}")],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| database_error("credential_reference", None, e))
    }

    pub(crate) fn set_credential_reference(
        &self,
        key: &str,
        reference: Option<&str>,
    ) -> Result<(), StorageError> {
        let key = format!("credential:{key}");
        match reference {
            Some(reference) => self.connection.execute(
                "INSERT INTO workspace_settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, reference],
            ),
            None => self
                .connection
                .execute("DELETE FROM workspace_settings WHERE key = ?1", [key]),
        }
        .map_err(|e| database_error("credential_reference", None, e))?;
        Ok(())
    }

    pub fn save_draft(
        &mut self,
        draft_id: &str,
        edit_revision: i64,
        body: &str,
    ) -> Result<i64, StorageError> {
        if draft_id.is_empty() || edit_revision < 0 {
            return Err(error(
                "INVALID_INPUT",
                "draft",
                None,
                false,
                "草稿标识或版本无效",
                "empty draft ID or negative revision",
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| database_error("draft", None, e))?;
        let current: Option<(i64, Option<String>)> = tx
            .query_row(
                "SELECT edit_revision, body FROM drafts WHERE id = ?1",
                [draft_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| database_error("draft", None, e))?;
        if let Some((revision, old_body)) = current {
            if edit_revision < revision
                || (edit_revision == revision && old_body.as_deref() != Some(body))
            {
                return Err(error(
                    "VERSION_CONFLICT",
                    "draft",
                    None,
                    false,
                    "草稿已有其他版本",
                    "draft revision is stale or has different content",
                ));
            }
            if edit_revision == revision {
                return Ok(revision);
            }
        }
        tx.execute(
            "INSERT INTO drafts (id, body, edit_revision, persisted_revision, updated_at)
             VALUES (?1, ?2, ?3, ?3, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
             ON CONFLICT(id) DO UPDATE SET body = excluded.body, edit_revision = excluded.edit_revision,
             persisted_revision = excluded.persisted_revision, updated_at = excluded.updated_at",
            params![draft_id, body, edit_revision],
        ).map_err(|e| database_error("draft", None, e))?;
        tx.commit().map_err(|e| database_error("draft", None, e))?;
        Ok(edit_revision)
    }

    pub fn load_draft(&self, draft_id: &str) -> Result<Option<Draft>, StorageError> {
        self.connection.query_row(
            "SELECT id, body, edit_revision, persisted_revision, updated_at FROM drafts WHERE id = ?1 AND body IS NOT NULL", [draft_id],
            |row| Ok(Draft { id: row.get(0)?, body: row.get(1)?, edit_revision: row.get(2)?, persisted_revision: row.get(3)?, updated_at: row.get(4)? }),
        ).optional().map_err(|e| database_error("draft", None, e))
    }

    pub(crate) fn draft_state(
        &self,
        draft_id: &str,
    ) -> Result<(i64, Option<String>), StorageError> {
        Ok(self
            .connection
            .query_row(
                "SELECT persisted_revision, body FROM drafts WHERE id = ?1",
                [draft_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| database_error("draft", None, e))?
            .unwrap_or((0, None)))
    }

    pub(crate) fn current_draft_intent(
        &self,
        draft_id: &str,
    ) -> Result<Option<(String, String, bool)>, StorageError> {
        self.connection
            .query_row(
                "SELECT i.request_id, i.relative_path, i.state = 'committed' FROM capture_intents i
                 JOIN drafts d ON i.draft_id = d.id AND i.draft_revision = d.persisted_revision
                 WHERE d.id = ?1 AND d.body IS NOT NULL
                 ORDER BY (i.state = 'committed') DESC, i.rowid LIMIT 1",
                [draft_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| database_error("recovery", None, e))
    }

    pub(crate) fn is_capture_path(&self, relative_path: &str) -> bool {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM capture_intents WHERE relative_path = ?1 AND state = 'committed')",
                [relative_path],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
    }

    pub(crate) fn capture_file_names(&self) -> Result<Vec<String>, StorageError> {
        let read_error = |error| database_error("capture_file_names", None, error);
        let mut statement = self
            .connection
            .prepare("SELECT file_name FROM capture_intents WHERE state = 'committed'")
            .map_err(read_error)?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(read_error)?
            .collect::<Result<_, _>>()
            .map_err(read_error)
    }

    pub(crate) fn list_recent_captures(&self) -> Result<Vec<RecentCapture>, StorageError> {
        let read_error = |error| database_error("recent_captures", None, error);
        let mut statement = self
            .connection
            .prepare(
                "SELECT i.relative_path, r.captured_at, r.id, r.source_id, CAST(s.snapshot AS TEXT), i.revision
                 FROM records r JOIN capture_intents i ON i.request_id = r.capture_request_id
                 LEFT JOIN source_revisions s ON s.source_id = r.source_id AND s.revision = i.revision
                 WHERE i.state = 'committed' AND r.lifecycle = 'active'
                 ORDER BY r.captured_at DESC, i.rowid DESC LIMIT ?1",
            )
            .map_err(read_error)?;
        let mut rows = statement
            .query([RECENT_CAPTURE_LIMIT])
            .map_err(read_error)?;
        let mut captures = Vec::new();
        while let Some(row) = rows.next().map_err(read_error)? {
            let record_id: String = row.get(2).map_err(read_error)?;
            let source_id: String = row.get(3).map_err(read_error)?;
            let snapshot: String = row.get(4).map_err(read_error)?;
            let body = snapshot
                .strip_prefix(&capture_header(&record_id, &source_id))
                .ok_or_else(|| {
                    error(
                        "WRITE_FAILED",
                        "recent_captures",
                        None,
                        false,
                        "捕获快照格式无效",
                        "original snapshot lacks its capture header",
                    )
                })?;
            captures.push((
                row.get(0).map_err(read_error)?,
                row.get(1).map_err(read_error)?,
                body.to_owned(),
                row.get(5).map_err(read_error)?,
            ));
        }
        Ok(captures)
    }

    pub fn save_record(
        &mut self,
        request_id: &str,
        draft_id: &str,
        draft_revision: i64,
        body: &str,
    ) -> Result<SavedRecord, StorageError> {
        if request_id.is_empty() || draft_id.is_empty() || draft_revision < 0 {
            return Err(error(
                "INVALID_INPUT",
                "prepare",
                Some(request_id),
                false,
                "保存请求无效",
                "empty ID or negative revision",
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| database_error("prepare", Some(request_id), e))?;
        let existing = read_intent(&tx, request_id)
            .map_err(|e| database_error("prepare", Some(request_id), e))?;
        let body_hash = revision_hash(body.as_bytes());
        if let Some(intent) = existing {
            if intent.draft_id != draft_id
                || intent.draft_revision != draft_revision
                || intent.body_hash != body_hash
            {
                return Err(error(
                    "VERSION_CONFLICT",
                    "prepare",
                    Some(request_id),
                    false,
                    "同一保存请求的内容不一致",
                    "request ID reused with different input",
                ));
            }
        } else {
            let persisted: Option<(i64, String)> = tx
                .query_row(
                    "SELECT persisted_revision, body FROM drafts WHERE id = ?1 AND body IS NOT NULL",
                    [draft_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|e| database_error("prepare", Some(request_id), e))?;
            if persisted
                .as_ref()
                .map(|(revision, text)| (*revision, text.as_str()))
                != Some((draft_revision, body))
            {
                return Err(error(
                    "VERSION_CONFLICT",
                    "prepare",
                    Some(request_id),
                    false,
                    "请先保存当前草稿",
                    "persisted draft does not match capture",
                ));
            }
            let (record_id, source_id): (String, String) = tx
                .query_row(
                    "SELECT lower(hex(randomblob(16))), lower(hex(randomblob(16)))",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|e| database_error("prepare", Some(request_id), e))?;
            let file_name = format!("{record_id}.md");
            let relative_path = if self.capture_dir.is_empty() {
                file_name.clone()
            } else {
                format!("{}/{file_name}", self.capture_dir)
            };
            let mut snapshot = capture_header(&record_id, &source_id);
            snapshot.push_str(body);
            let bytes = snapshot.into_bytes();
            let revision = revision_hash(&bytes);
            tx.execute(
                "INSERT INTO capture_intents (request_id, draft_id, draft_revision, body_hash, record_id, source_id, file_name, relative_path, file_bytes, revision, captured_at, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, strftime('%Y-%m-%dT%H:%M:%fZ','now'), 'prepared')",
                params![request_id, draft_id, draft_revision, body_hash, record_id, source_id, file_name, relative_path, bytes, revision],
            ).map_err(|e| database_error("prepare", Some(request_id), e))?;
        }
        tx.commit()
            .map_err(|e| database_error("prepare", Some(request_id), e))?;
        #[cfg(test)]
        crash_at("prepared");
        self.complete_intent(request_id)
    }

    pub fn recover_pending(&mut self) -> Result<Vec<RecoveryOutcome>, StorageError> {
        let requests = {
            let mut statement = self.connection.prepare("SELECT request_id FROM capture_intents WHERE state != 'committed' ORDER BY rowid")
                .map_err(|e| database_error("recovery", None, e))?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| database_error("recovery", None, e))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| database_error("recovery", None, e))?
        };
        Ok(requests
            .into_iter()
            .map(|request_id| {
                let result = self.complete_intent(&request_id);
                RecoveryOutcome { request_id, result }
            })
            .collect())
    }

    pub fn clear_committed_draft(&mut self, request_id: &str) -> Result<bool, StorageError> {
        let count = self
            .connection
            .execute(
                "UPDATE drafts SET body = NULL WHERE body IS NOT NULL AND EXISTS
             (SELECT 1 FROM capture_intents i WHERE i.request_id = ?1 AND i.state = 'committed'
              AND i.draft_id = drafts.id AND i.draft_revision = drafts.persisted_revision)",
                [request_id],
            )
            .map_err(|e| database_error("draft_confirmation", Some(request_id), e))?;
        Ok(count == 1)
    }

    fn complete_intent(&mut self, request_id: &str) -> Result<SavedRecord, StorageError> {
        let intent = read_intent(&self.connection, request_id)
            .map_err(|e| database_error("recovery", Some(request_id), e))?
            .ok_or_else(|| {
                error(
                    "INVALID_INPUT",
                    "recovery",
                    Some(request_id),
                    false,
                    "保存请求不存在",
                    "capture intent missing",
                )
            })?;
        if intent.state == "committed" {
            return Ok(saved_record(&intent));
        }
        let bytes = intent
            .bytes
            .as_deref()
            .filter(|bytes| revision_hash(bytes) == intent.revision)
            .ok_or_else(|| {
                error(
                    "VERSION_CONFLICT",
                    "recovery",
                    Some(request_id),
                    false,
                    "恢复内容与保存版本不一致",
                    "capture intent bytes do not match the recorded revision",
                )
            })?;
        let file = match self.files.publish_record(&intent.relative_path, bytes) {
            Ok(file) => file,
            Err(e) => {
                let conflict = e.kind() == io::ErrorKind::AlreadyExists;
                let detail = match self.connection.execute(
                    "UPDATE capture_intents SET state = ?2 WHERE request_id = ?1",
                    params![request_id, if conflict { "conflict" } else { "failed" }],
                ) {
                    Ok(_) => e.to_string(),
                    Err(database_error) => {
                        format!("{e}. Failed to record status: {database_error}")
                    }
                };
                return Err(error(
                    if conflict {
                        "VERSION_CONFLICT"
                    } else {
                        "WRITE_FAILED"
                    },
                    "publish",
                    Some(request_id),
                    !conflict,
                    if conflict {
                        "目标文件与保存请求冲突"
                    } else {
                        "记录文件写入失败"
                    },
                    detail,
                ));
            }
        };
        #[cfg(test)]
        crash_at("published");
        self.connection
            .execute(
                "UPDATE capture_intents SET state = 'file_written' WHERE request_id = ?1",
                [request_id],
            )
            .map_err(|e| database_error("file_written", Some(request_id), e))?;
        #[cfg(test)]
        crash_at("file_written");
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| database_error("commit", Some(request_id), e))?;
        let saved_at: String = tx
            .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
                row.get(0)
            })
            .map_err(|e| database_error("commit", Some(request_id), e))?;
        tx.execute("INSERT INTO sources (id, location, current_revision, availability) VALUES (?1, ?2, ?3, 'available')",
            params![intent.source_id, intent.relative_path, intent.revision]).map_err(|e| database_error("commit", Some(request_id), e))?;
        tx.execute("INSERT INTO source_revisions (source_id, revision, observed_at, snapshot, encoding, newline) VALUES (?1, ?2, ?3, ?4, 'utf-8', ?5)",
            params![intent.source_id, intent.revision, saved_at, bytes, newline(bytes)])
            .map_err(|e| database_error("commit", Some(request_id), e))?;
        tx.execute("INSERT INTO records (id, capture_request_id, source_id, kind, captured_at, lifecycle, origin) VALUES (?1, ?2, ?3, 'note', ?4, 'active', 'fresh_capture')",
            params![intent.record_id, request_id, intent.source_id, intent.captured_at]).map_err(|e| database_error("commit", Some(request_id), e))?;
        tx.execute(
            "INSERT INTO jobs (id, operation_key, kind, source_id, source_revision, state, attempts, deletion_epoch)
             VALUES (lower(hex(randomblob(16))), ?1, 'index', ?2, ?3, 'queued', 0, 0)",
            params![
                format!("{}:{}:index", intent.source_id, intent.revision),
                intent.source_id,
                intent.revision
            ],
        )
        .map_err(|e| database_error("commit", Some(request_id), e))?;
        tx.execute(
            "UPDATE capture_intents SET state = 'committed', saved_at = ?2, file_bytes = NULL WHERE request_id = ?1",
            params![request_id, saved_at],
        )
        .map_err(|e| database_error("commit", Some(request_id), e))?;
        tx.commit()
            .map_err(|e| database_error("commit", Some(request_id), e))?;
        #[cfg(test)]
        crash_at("committed");
        drop(file);
        Ok(SavedRecord {
            record_id: intent.record_id,
            source_id: intent.source_id,
            source_revision: intent.revision,
            relative_path: intent.relative_path,
            saved_at,
        })
    }
}

fn capture_header(record_id: &str, source_id: &str) -> String {
    format!("---\nrecord_id: {record_id}\nsource_id: {source_id}\n---\n")
}

fn saved_record(intent: &Intent) -> SavedRecord {
    SavedRecord {
        record_id: intent.record_id.clone(),
        source_id: intent.source_id.clone(),
        source_revision: intent.revision.clone(),
        relative_path: intent.relative_path.clone(),
        saved_at: intent
            .saved_at
            .clone()
            .expect("committed intent has saved_at"),
    }
}

fn read_intent(connection: &Connection, request_id: &str) -> rusqlite::Result<Option<Intent>> {
    connection.query_row("SELECT draft_id, draft_revision, body_hash, record_id, source_id, file_name, relative_path, file_bytes, revision, captured_at, state, saved_at
        FROM capture_intents WHERE request_id = ?1", [request_id], |row| Ok(Intent {
        draft_id: row.get(0)?, draft_revision: row.get(1)?, body_hash: row.get(2)?,
        record_id: row.get(3)?, source_id: row.get(4)?, file_name: row.get(5)?, relative_path: row.get(6)?,
        bytes: row.get(7)?, revision: row.get(8)?, captured_at: row.get(9)?, state: row.get(10)?, saved_at: row.get(11)?,
    })).optional()
}

fn capture_directory(path: &Path) -> Result<String, StorageError> {
    if path.as_os_str().is_empty() {
        return Err(error(
            "INVALID_INPUT",
            "open",
            None,
            false,
            "请选择保存目录",
            "capture path is empty",
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => (),
            std::path::Component::Normal(part) => parts.push(part.to_str().ok_or_else(|| {
                error(
                    "INVALID_INPUT",
                    "open",
                    None,
                    false,
                    "保存目录无法保存",
                    "capture path is not UTF-8",
                )
            })?),
            _ => {
                return Err(error(
                    "OUT_OF_SCOPE",
                    "open",
                    None,
                    false,
                    "保存目录必须位于工作区内",
                    "capture directory is not a relative path",
                ));
            }
        }
    }
    Ok(parts.join("/"))
}

fn verify_capture_directory(requested: &str, persisted: &str) -> Result<(), StorageError> {
    if requested != persisted {
        return Err(error(
            "VERSION_CONFLICT",
            "open",
            None,
            false,
            "保存目录与现有工作区不一致",
            "capture directory differs from persisted setting",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn crash_at(stage: &str) {
    if std::env::var("FLASH_STORAGE_TEST_CRASH").as_deref() == Ok(stage) {
        std::process::exit(86);
    }
}

fn newline(bytes: &[u8]) -> &'static str {
    let crlf = bytes.windows(2).filter(|pair| *pair == b"\r\n").count();
    let lf = bytes.iter().filter(|byte| **byte == b'\n').count();
    if crlf == 0 {
        "lf"
    } else if crlf == lf {
        "crlf"
    } else {
        "mixed"
    }
}

fn revision_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const APPLICATION_ID: i64 = i64::from_be_bytes(*b"\0\0\0\0FMD1");
const SCHEMA_VERSION: i64 = 4;
const SCHEMA: &str = "
BEGIN IMMEDIATE;
CREATE TABLE workspace_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE drafts (id TEXT PRIMARY KEY, body TEXT, edit_revision INTEGER NOT NULL CHECK(edit_revision >= 0), persisted_revision INTEGER NOT NULL CHECK(persisted_revision = edit_revision), updated_at TEXT NOT NULL);
CREATE TABLE capture_intents (request_id TEXT PRIMARY KEY, draft_id TEXT NOT NULL, draft_revision INTEGER NOT NULL, body_hash TEXT NOT NULL, record_id TEXT NOT NULL UNIQUE, source_id TEXT NOT NULL UNIQUE, file_name TEXT NOT NULL UNIQUE, relative_path TEXT NOT NULL UNIQUE, file_bytes BLOB, revision TEXT NOT NULL, captured_at TEXT NOT NULL, state TEXT NOT NULL CHECK(state IN ('prepared','file_written','committed','failed','conflict')), saved_at TEXT, CHECK((state = 'committed') = (saved_at IS NOT NULL)), CHECK((state = 'committed') = (file_bytes IS NULL)));
CREATE TABLE sources (id TEXT PRIMARY KEY, location TEXT NOT NULL UNIQUE, current_revision TEXT NOT NULL, availability TEXT NOT NULL);
CREATE TABLE source_revisions (source_id TEXT NOT NULL REFERENCES sources(id), revision TEXT NOT NULL, observed_at TEXT NOT NULL, snapshot BLOB NOT NULL, encoding TEXT NOT NULL, newline TEXT NOT NULL, PRIMARY KEY(source_id, revision));
CREATE TABLE records (id TEXT PRIMARY KEY, capture_request_id TEXT NOT NULL UNIQUE REFERENCES capture_intents(request_id), source_id TEXT NOT NULL UNIQUE REFERENCES sources(id), kind TEXT NOT NULL, captured_at TEXT NOT NULL, lifecycle TEXT NOT NULL, origin TEXT NOT NULL);
CREATE TABLE jobs (id TEXT PRIMARY KEY, operation_key TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, source_id TEXT NOT NULL, source_revision TEXT NOT NULL, state TEXT NOT NULL, attempts INTEGER NOT NULL, deletion_epoch INTEGER NOT NULL, FOREIGN KEY(source_id, source_revision) REFERENCES source_revisions(source_id, revision));";

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct TestDir(pub(crate) PathBuf);

    impl TestDir {
        pub(crate) fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "flash-storage-test-{}-{stamp}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        fn open(&self) -> Storage {
            Storage::open(&self.0, Path::new("."), ".flash-test").unwrap()
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn crash_child() {
        let Some(root) = std::env::var_os("FLASH_STORAGE_TEST_ROOT") else {
            return;
        };
        let mut storage = Storage::open(Path::new(&root), Path::new("."), ".flash-test").unwrap();
        storage.save_draft("draft", 1, "正文\r\n原样保留").unwrap();
        storage
            .save_record("request", "draft", 1, "正文\r\n原样保留")
            .unwrap();
        panic!("crash point was not reached");
    }

    fn crash_save(root: &Path, stage: &str) {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "storage::tests::crash_child", "--nocapture"])
            .env("FLASH_STORAGE_TEST_ROOT", root)
            .env("FLASH_STORAGE_TEST_CRASH", stage)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));
    }

    #[test]
    fn recovers_after_process_exit_at_each_boundary() {
        for stage in ["prepared", "published", "file_written", "committed"] {
            let dir = TestDir::new();
            crash_save(&dir.0, stage);
            let mut storage = dir.open();
            let outcomes = storage.recover_pending().unwrap();
            if stage == "committed" {
                assert!(outcomes.is_empty());
            } else {
                assert_eq!(outcomes.len(), 1);
                assert!(outcomes.into_iter().next().unwrap().result.is_ok());
            }
            let saved = storage
                .save_record("request", "draft", 1, "正文\r\n原样保留")
                .unwrap();
            assert_eq!(
                saved,
                storage
                    .save_record("request", "draft", 1, "正文\r\n原样保留")
                    .unwrap()
            );
            let bytes = fs::read(dir.0.join(&saved.relative_path)).unwrap();
            assert!(bytes.ends_with("正文\r\n原样保留".as_bytes()));
            assert_eq!(saved.source_revision, revision_hash(&bytes));
            let records: i64 = storage
                .connection
                .query_row("SELECT count(*) FROM records", [], |row| row.get(0))
                .unwrap();
            let jobs: i64 = storage
                .connection
                .query_row("SELECT count(*) FROM jobs", [], |row| row.get(0))
                .unwrap();
            assert_eq!((records, jobs), (1, 1));
            assert!(storage.load_draft("draft").unwrap().is_some());
        }
    }

    #[test]
    fn same_body_with_distinct_requests_creates_distinct_records() {
        let dir = TestDir::new();
        let mut storage = dir.open();
        storage.save_draft("draft", 1, "相同正文").unwrap();
        let first = storage.save_record("one", "draft", 1, "相同正文").unwrap();
        let second = storage.save_record("two", "draft", 1, "相同正文").unwrap();
        assert_ne!(first.record_id, second.record_id);
        assert_ne!(first.relative_path, second.relative_path);
        assert_eq!(
            storage
                .save_record("one", "draft", 1, "其他正文")
                .unwrap_err()
                .code,
            "VERSION_CONFLICT"
        );
    }

    #[test]
    fn recent_captures_return_bounded_original_snapshots_in_capture_order() {
        let dir = TestDir::new();
        let mut storage = dir.open();
        assert!(storage.list_recent_captures().unwrap().is_empty());
        let captured_at = "2026-11-18T00:00:00.000Z";
        let mut expected = Vec::new();
        for revision in 1..=RECENT_CAPTURE_LIMIT {
            let body = format!("记录 {revision}");
            storage.save_draft("draft", revision, &body).unwrap();
            let saved = storage
                .save_record(&format!("capture-{revision}"), "draft", revision, &body)
                .unwrap();
            expected.push((
                saved.relative_path,
                captured_at.to_owned(),
                body,
                saved.source_revision,
            ));
        }
        let latest_revision = RECENT_CAPTURE_LIMIT + 1;
        let latest_body = "---\r\n标题: 笔记\r\n---\r\n记录正文";
        storage
            .save_draft("draft", latest_revision, latest_body)
            .unwrap();
        let latest = storage
            .save_record("latest", "draft", latest_revision, latest_body)
            .unwrap();
        storage
            .connection
            .execute("UPDATE records SET captured_at = ?1", [captured_at])
            .unwrap();
        fs::write(dir.0.join(&latest.relative_path), "后续修改").unwrap();
        storage.connection.execute(
            "INSERT INTO source_revisions (source_id, revision, observed_at, snapshot, encoding, newline)
             VALUES (?1, 'later-edit', ?2, ?3, 'utf-8', 'lf')",
            params![latest.source_id, captured_at, "后续修改".as_bytes()],
        ).unwrap();
        storage
            .connection
            .execute(
                "UPDATE sources SET current_revision = 'later-edit' WHERE id = ?1",
                [&latest.source_id],
            )
            .unwrap();

        expected.push((
            latest.relative_path,
            captured_at.to_owned(),
            latest_body.to_owned(),
            latest.source_revision,
        ));
        expected.reverse();
        expected.pop();
        assert_eq!(storage.list_recent_captures().unwrap(), expected);
    }

    #[test]
    fn stale_drafts_and_confirmation_do_not_remove_newer_edits() {
        let dir = TestDir::new();
        let mut storage = dir.open();
        storage.save_draft("draft", 1, "旧").unwrap();
        storage.save_record("one", "draft", 1, "旧").unwrap();
        storage.save_draft("draft", 2, "新").unwrap();
        assert_eq!(
            storage.save_draft("draft", 1, "旧").unwrap_err().code,
            "VERSION_CONFLICT"
        );
        assert_eq!(
            storage.save_draft("draft", 2, "不同").unwrap_err().code,
            "VERSION_CONFLICT"
        );
        assert!(!storage.clear_committed_draft("one").unwrap());
        assert_eq!(storage.load_draft("draft").unwrap().unwrap().body, "新");
        storage.save_record("two", "draft", 2, "新").unwrap();
        assert!(storage.clear_committed_draft("two").unwrap());
        assert!(storage.load_draft("draft").unwrap().is_none());
        assert_eq!(
            storage.save_draft("draft", 1, "旧").unwrap_err().code,
            "VERSION_CONFLICT"
        );
        assert_eq!(
            storage.save_draft("draft", 2, "新").unwrap_err().code,
            "VERSION_CONFLICT"
        );
        assert!(storage.load_draft("draft").unwrap().is_none());
        assert_eq!(storage.save_draft("draft", 3, "更新").unwrap(), 3);
    }

    #[test]
    fn conflict_preserves_file_and_draft_until_resolved() {
        let dir = TestDir::new();
        crash_save(&dir.0, "prepared");
        let mut storage = dir.open();
        let name: String = storage
            .connection
            .query_row(
                "SELECT file_name FROM capture_intents WHERE request_id = 'request'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let path = dir.0.join(name);
        fs::write(&path, b"other file").unwrap();
        let outcome = storage.recover_pending().unwrap().pop().unwrap();
        assert_eq!(outcome.result.unwrap_err().code, "VERSION_CONFLICT");
        assert_eq!(fs::read(&path).unwrap(), b"other file");
        assert!(storage.load_draft("draft").unwrap().is_some());
        fs::remove_file(path).unwrap();
        assert!(
            storage
                .recover_pending()
                .unwrap()
                .pop()
                .unwrap()
                .result
                .is_ok()
        );
    }

    #[test]
    fn updates_capture_directory_and_rejects_unrecognized_database() {
        let dir = TestDir::new();
        fs::create_dir(dir.0.join("other")).unwrap();
        {
            let mut first = dir.open();
            first.save_draft("draft", 1, "旧目录笔记").unwrap();
            let saved = first
                .save_record("first", "draft", 1, "旧目录笔记")
                .unwrap();
            assert!(!saved.relative_path.contains('/'));
        }
        let mut updated = Storage::open(&dir.0, Path::new("other"), ".flash-test").unwrap();
        assert_eq!(updated.capture_dir(), "other");
        updated.save_draft("draft", 2, "新目录笔记").unwrap();
        let next = updated
            .save_record("second", "draft", 2, "新目录笔记")
            .unwrap();
        assert!(next.relative_path.starts_with("other/"));
        assert!(dir.0.join(&next.relative_path).is_file());
        assert!(updated.update_capture_dir(Path::new("missing")).is_err());
        assert_eq!(updated.capture_dir(), "other");
        drop(updated);

        let other = TestDir::new();
        fs::create_dir(other.0.join(".flash-test")).unwrap();
        let db = Connection::open(other.0.join(".flash-test").join("state.sqlite")).unwrap();
        db.execute("CREATE TABLE unrelated (value TEXT)", [])
            .unwrap();
        drop(db);
        assert_eq!(
            Storage::open(&other.0, Path::new("."), ".flash-test")
                .err()
                .unwrap()
                .code,
            "CAPABILITY_UNSUPPORTED"
        );
        let db = Connection::open(other.0.join(".flash-test").join("state.sqlite")).unwrap();
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = 'unrelated'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn workspace_lock_and_identity_are_scoped_to_the_workspace() {
        let first = TestDir::new();
        let second = TestDir::new();
        let storage = first.open();
        let other = second.open();
        let first_id = storage.workspace_id().to_owned();
        assert_ne!(first_id, other.workspace_id());
        assert!(Storage::open(&first.0, Path::new("."), ".flash-test").is_err());
        assert!(fs::rename(first.0.join(".flash-test"), first.0.join("moved-state")).is_err());
        drop(storage);
        assert_eq!(first.open().workspace_id(), first_id);
        assert!(Storage::open(&first.0, Path::new(".FLASH-TEST"), ".flash-test").is_err());
        assert!(Storage::open(&first.0, Path::new(".."), ".flash-test").is_err());
    }

    #[test]
    fn write_failure_preserves_draft_and_resumes_partial_stage() {
        let dir = TestDir::new();
        crash_save(&dir.0, "prepared");
        let mut storage = dir.open();
        let capture_time = "2026-01-01T00:00:00Z";
        storage
            .connection
            .execute(
                "UPDATE capture_intents SET captured_at = ?1",
                [capture_time],
            )
            .unwrap();
        let intent = read_intent(&storage.connection, "request")
            .unwrap()
            .unwrap();
        let bytes = intent.bytes.unwrap();
        let stage = dir.0.join(format!(".{}.pending", intent.file_name));
        fs::write(&stage, &bytes[..bytes.len() / 2]).unwrap();
        let locked = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&stage)
            .unwrap();
        let outcome = storage.recover_pending().unwrap().pop().unwrap();
        assert_eq!(outcome.result.unwrap_err().code, "WRITE_FAILED");
        assert!(storage.load_draft("draft").unwrap().is_some());
        assert!(!dir.0.join(&intent.file_name).exists());
        drop(locked);
        assert!(
            storage
                .recover_pending()
                .unwrap()
                .pop()
                .unwrap()
                .result
                .is_ok()
        );
        assert_eq!(fs::read(dir.0.join(&intent.file_name)).unwrap(), bytes);
        assert!(!stage.exists());
        let captured_at: String = storage
            .connection
            .query_row("SELECT captured_at FROM records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(captured_at, capture_time);
    }

    #[test]
    fn bundled_sqlite_supports_chinese_substrings_and_fts5_trigrams() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE VIRTUAL TABLE notes USING fts5(body, tokenize='trigram');
                INSERT INTO notes (body) VALUES ('读书笔记桌面设计');",
            )
            .unwrap();
        for term in ["读", "读书", "读书笔"] {
            let count: i64 = connection
                .query_row(
                    "SELECT count(*) FROM notes WHERE instr(body, ?1) > 0",
                    [term],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }
        let match_count: i64 = connection
            .query_row(
                "SELECT count(*) FROM notes WHERE notes MATCH ?1",
                ["读书笔"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(match_count, 1);
        let version: String = connection
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .unwrap();
        println!("Bundled SQLite {version}");
    }
}
