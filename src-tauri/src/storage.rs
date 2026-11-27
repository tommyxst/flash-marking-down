use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, Prefix};
use std::sync::{Arc, Mutex};
use std::{fs, io};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::dto::{NoteDetailDto, NoteSummaryDto, OrganizeResultDto};
use crate::workspace::{
    DEFAULT_CAPTURE_DIR, DEFAULT_RAW_DIR, WorkspaceFiles, normalize_workspace_subdir,
    replace_markdown,
};

mod search;
pub(crate) use search::SearchReader;
pub(crate) mod m2;
pub(crate) mod m2_answer;
pub(crate) mod memory_sync;
pub(crate) mod wiki;
pub(crate) mod wikra;

const RECENT_CAPTURE_LIMIT: i64 = 50;
const NOTE_LIST_LIMIT: i64 = 200;
const UPSERT_WORKSPACE_SETTING: &str = "INSERT INTO workspace_settings (key, value) VALUES (?1, ?2)
    ON CONFLICT(key) DO UPDATE SET value = excluded.value WHERE value <> excluded.value";

pub(crate) type RecentCapture = (String, String, String, String, Option<String>);
pub(crate) const NOTE_ANCHOR: &str = "#fmd-note:";

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

pub struct Storage {
    connection: Connection,
    workspace: WorkspaceStorage,
}

#[derive(Clone)]
pub(crate) struct WorkspaceStorage {
    files: Arc<WorkspaceFiles>,
    search_build: Arc<Mutex<()>>,
    raw_dir: String,
    capture_dir: String,
    workspace_id: String,
}

impl WorkspaceStorage {
    pub(crate) fn database_path(&self) -> &Path {
        self.files.database_path()
    }

    pub(crate) fn update_dirs(&mut self, raw_dir: String, capture_dir: String) {
        self.raw_dir = raw_dir;
        self.capture_dir = capture_dir;
    }

    pub(crate) fn with_storage<T, E: From<StorageError>>(
        &self,
        action: impl FnOnce(&mut Storage) -> Result<T, E>,
    ) -> Result<T, E> {
        let connection = self.connect()?;
        let mut storage = Storage {
            connection,
            workspace: self.clone(),
        };
        action(&mut storage)
    }

    fn connect(&self) -> Result<Connection, StorageError> {
        let path = self.files.existing_database_path().map_err(|e| {
            error(
                "WRITE_FAILED",
                "workspace",
                None,
                true,
                "无法访问工作区状态库，请重新打开工作区",
                e,
            )
        })?;
        connect(path, false)
    }
}

fn connect(path: &Path, create: bool) -> Result<Connection, StorageError> {
    let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    if create {
        flags |= OpenFlags::SQLITE_OPEN_CREATE;
    }
    let connection =
        Connection::open_with_flags(path, flags).map_err(|e| database_error("open", None, e))?;
    connection
        .execute_batch(
            "PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;",
        )
        .map_err(|e| database_error("open", None, e))?;
    Ok(connection)
}

fn is_unc_path(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
    )
}

impl Storage {
    pub fn open(
        root: &Path,
        capture_dir: &Path,
        state_dir_name: &str,
    ) -> Result<Self, StorageError> {
        let open_error = |e| error("WRITE_FAILED", "workspace", None, true, "无法打开工作区", e);
        let files = WorkspaceFiles::open(root, state_dir_name).map_err(open_error)?;
        let capture_dir = if capture_dir.as_os_str().is_empty() || capture_dir == Path::new(".") {
            Path::new(DEFAULT_CAPTURE_DIR)
        } else {
            capture_dir
        };
        let capture_dir = files.capture_key(capture_dir).map_err(open_error)?;
        let mut connection = connect(files.database_path(), true)?;

        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(migration_error)?;
        let application_id: i64 = connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(migration_error)?;
        let empty_schema = connection
            .query_row("SELECT count(*) = 0 FROM sqlite_schema", [], |row| {
                row.get::<_, bool>(0)
            })
            .map_err(migration_error)?;
        let is_new = version == 0 && application_id == 0 && empty_schema;
        let is_existing =
            (1..=SCHEMA_VERSION).contains(&version) && application_id == APPLICATION_ID;
        if !is_new && !is_existing {
            return Err(error(
                "CAPABILITY_UNSUPPORTED",
                "migration",
                None,
                false,
                "工作区状态库版本不受支持",
                version,
            ));
        }

        // Switch journal_mode before starting any write transaction (especially for WAL-on-SMB).
        connection
            .pragma_update(
                None,
                "journal_mode",
                if is_unc_path(files.database_path()) {
                    "DELETE"
                } else {
                    "WAL"
                },
            )
            .map_err(|e| database_error("open", None, e))?;

        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(migration_error)?;
        let version: i64 = tx
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(migration_error)?;
        let application_id: i64 = tx
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(migration_error)?;
        match version {
            0 if application_id == 0
                && tx
                    .query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .map_err(migration_error)?
                    == 0 =>
            {
                tx.execute_batch(SCHEMA).map_err(migration_error)?;
                tx.execute_batch(search::SCHEMA).map_err(migration_error)?;
                tx.execute_batch(wiki::SCHEMA).map_err(migration_error)?;
                tx.execute_batch(m2_answer::SCHEMA)
                    .map_err(migration_error)?;
                tx.execute_batch(NOTES_SCHEMA).map_err(migration_error)?;
                tx.execute_batch(wikra::SCHEMA).map_err(migration_error)?;
                tx.execute_batch(wikra::MIGRATE_V8)
                    .map_err(migration_error)?;
                tx.pragma_update(None, "application_id", APPLICATION_ID)
                    .map_err(migration_error)?;
                tx.execute(
                    "INSERT INTO workspace_settings VALUES ('id', lower(hex(randomblob(16))))",
                    [],
                )
                .map_err(migration_error)?;
                tx.pragma_update(None, "user_version", SCHEMA_VERSION)
                    .map_err(migration_error)?;
            }
            1..=SCHEMA_VERSION if application_id == APPLICATION_ID => {
                if version < 5 {
                    migrate_workspace(&tx, version)?;
                    tx.execute_batch(NOTES_SCHEMA).map_err(migration_error)?;
                    migrate_captures(&tx)?;
                }
                if version < 6 {
                    migrate_to_v6(&tx)?;
                }
                if version < 7 {
                    tx.execute_batch(wikra::SCHEMA).map_err(migration_error)?;
                }
                if version < 8 {
                    tx.execute_batch(wikra::MIGRATE_V8)
                        .map_err(migration_error)?;
                    tx.pragma_update(None, "user_version", SCHEMA_VERSION)
                        .map_err(migration_error)?;
                }
            }
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
        tx.execute(
            UPSERT_WORKSPACE_SETTING,
            params!["capture_dir", &capture_dir],
        )
        .map_err(migration_error)?;
        let raw_dir = tx
            .query_row(
                "SELECT value FROM workspace_settings WHERE key = 'raw_dir'",
                [],
                |row| {
                    Ok(row
                        .get_ref(0)?
                        .as_str()
                        .ok()
                        .and_then(|value| normalize_workspace_subdir(Path::new(value)).ok()))
                },
            )
            .optional()
            .map_err(migration_error)?
            .flatten()
            .unwrap_or_else(|| DEFAULT_RAW_DIR.to_owned());
        let workspace_id = tx
            .query_row(
                "SELECT value FROM workspace_settings WHERE key = 'id'",
                [],
                |row| row.get(0),
            )
            .map_err(migration_error)?;
        tx.commit().map_err(migration_error)?;
        connection
            .execute(
                "UPDATE background_jobs SET status = 'queued' WHERE status = 'running'",
                [],
            )
            .map_err(|e| database_error("recover_jobs", None, e))?;
        Ok(Self {
            connection,
            workspace: WorkspaceStorage {
                files: Arc::new(files),
                search_build: Arc::default(),
                raw_dir,
                capture_dir,
                workspace_id,
            },
        })
    }

    pub(crate) fn into_workspace(self) -> WorkspaceStorage {
        self.workspace
    }

    pub fn workspace_id(&self) -> &str {
        &self.workspace.workspace_id
    }

    pub(crate) fn raw_dir(&self) -> &str {
        &self.workspace.raw_dir
    }

    pub(crate) fn clips_dir(&self) -> String {
        format!("{}/clips", self.raw_dir())
    }

    pub(crate) fn capture_dir(&self) -> &str {
        &self.workspace.capture_dir
    }

    pub(crate) fn set_workspace_dirs(
        &mut self,
        raw_dir: &Path,
        capture_dir: Option<&Path>,
    ) -> Result<(String, String), StorageError> {
        let dir_error = |err| {
            error(
                "WRITE_FAILED",
                "workspace",
                None,
                true,
                "无法使用指定的工作区子目录，请检查路径和访问权限",
                err,
            )
        };
        let raw_dir = self
            .workspace
            .files
            .capture_key(raw_dir)
            .map_err(dir_error)?;
        let capture_dir = capture_dir
            .map(|path| self.workspace.files.capture_key(path))
            .transpose()
            .map_err(dir_error)?
            .unwrap_or_else(|| self.workspace.capture_dir.clone());

        let db = |err| database_error("workspace", None, err);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute(UPSERT_WORKSPACE_SETTING, params!["raw_dir", &raw_dir])
            .map_err(db)?;
        if capture_dir != self.workspace.capture_dir {
            tx.execute(
                UPSERT_WORKSPACE_SETTING,
                params!["capture_dir", &capture_dir],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        self.workspace
            .update_dirs(raw_dir.clone(), capture_dir.clone());
        Ok((raw_dir, capture_dir))
    }

    pub(crate) fn workspace_files(&self) -> Arc<WorkspaceFiles> {
        self.workspace.files.clone()
    }

    pub(crate) fn workspace_handle(&self) -> WorkspaceStorage {
        self.workspace.clone()
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

    pub(crate) fn committed_draft(
        &self,
        draft_id: &str,
    ) -> Result<Option<(String, String)>, StorageError> {
        self.connection
            .query_row(
                "SELECT n.request_id, n.relative_path FROM notes n JOIN drafts d
                ON n.draft_id = d.id AND n.draft_revision = d.persisted_revision
             WHERE d.id = ?1 AND d.body IS NOT NULL ORDER BY n.rowid DESC LIMIT 1",
                [draft_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| database_error("draft_confirmation", None, e))
    }

    pub(crate) fn is_capture_path(&self, relative_path: &str) -> bool {
        is_capture(&self.connection, relative_path)
    }

    pub(crate) fn is_in_capture_dir(&self, key: &str) -> bool {
        is_in_capture_dir(key, self.capture_dir())
    }

    pub(crate) fn capture_paths(&self) -> Result<HashSet<String>, StorageError> {
        let read_error = |error| database_error("capture_paths", None, error);
        let mut q = self
            .connection
            .prepare("SELECT DISTINCT relative_path FROM notes WHERE draft_id <> 'manual'")
            .map_err(read_error)?;
        q.query_map([], |row| row.get(0))
            .map_err(read_error)?
            .collect::<Result<_, _>>()
            .map_err(read_error)
    }

    pub(crate) fn list_recent_captures(&self) -> Result<Vec<RecentCapture>, StorageError> {
        let read_error = |error| database_error("recent_captures", None, error);
        let mut statement = self
            .connection
            .prepare(
                "SELECT relative_path, captured_at, id, source_id, block_id, content_hash
                 FROM notes
                 WHERE draft_id <> 'manual'
                 ORDER BY captured_at DESC, rowid DESC LIMIT ?1",
            )
            .map_err(read_error)?;
        let mut rows = statement
            .query([RECENT_CAPTURE_LIMIT])
            .map_err(read_error)?;
        let mut files = HashMap::new();
        let mut captures = Vec::new();
        while let Some(row) = rows.next().map_err(read_error)? {
            let path: String = row.get(0).map_err(read_error)?;
            let block_id: Option<String> = row.get(4).map_err(read_error)?;
            let cached = files.entry(path.clone()).or_insert_with(|| {
                self.workspace
                    .files
                    .read_markdown(&path)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
            });
            let Some(text) = cached.as_deref() else {
                continue;
            };
            let (body, revision) = if let Some(id) = &block_id {
                let Some(body) = block_body(text, id) else {
                    continue;
                };
                (body.to_owned(), revision_hash(body.as_bytes()))
            } else {
                let record: String = row.get(2).map_err(read_error)?;
                let source: String = row.get(3).map_err(read_error)?;
                let stripped = text
                    .strip_prefix(&capture_header(&record, &source))
                    .unwrap_or(text);
                let hash = revision_hash(text.as_bytes());
                (stripped.to_owned(), hash)
            };
            captures.push((
                path,
                row.get(1).map_err(read_error)?,
                body,
                revision,
                block_id,
            ));
        }
        Ok(captures)
    }

    pub(crate) fn list_notes(&self) -> Result<Vec<NoteSummaryDto>, StorageError> {
        let read_error = |error| database_error("notes", None, error);
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, source_id, relative_path, block_id, captured_at, title, body_preview,
                        content_hash, organize_status, ov_resource_status, ov_memory_status
                 FROM notes
                 WHERE draft_id <> 'manual'
                 ORDER BY captured_at DESC, rowid DESC LIMIT ?1",
            )
            .map_err(read_error)?;
        let mut rows = statement.query([NOTE_LIST_LIMIT]).map_err(read_error)?;
        let mut files = HashMap::new();
        let mut notes = Vec::new();
        while let Some(row) = rows.next().map_err(read_error)? {
            let id: String = row.get(0).map_err(read_error)?;
            let source_id: String = row.get(1).map_err(read_error)?;
            let path: String = row.get(2).map_err(read_error)?;
            let block_id: Option<String> = row.get(3).map_err(read_error)?;
            let captured_at: String = row.get(4).map_err(read_error)?;
            let fallback_title: String = row.get(5).map_err(read_error)?;
            let fallback_preview: String = row.get(6).map_err(read_error)?;
            let fallback_hash: String = row.get(7).map_err(read_error)?;
            let organize_status: String = row.get(8).map_err(read_error)?;
            let ov_resource_status: String = row.get(9).map_err(read_error)?;
            let ov_memory_status: String = row.get(10).map_err(read_error)?;

            let cached = files.entry(path.clone()).or_insert_with(|| {
                self.workspace
                    .files
                    .read_markdown(&path)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
            });
            let (title, body_preview, content_hash) = if let Some(text) = cached.as_deref() {
                if let Some(marker) = block_id.as_deref() {
                    let Some(body) = block_body(text, marker) else {
                        continue;
                    };
                    (
                        note_title(body),
                        note_preview(body),
                        revision_hash(body.as_bytes()),
                    )
                } else {
                    let body = text
                        .strip_prefix(&capture_header(&id, &source_id))
                        .unwrap_or(text);
                    (
                        note_title(body),
                        note_preview(body),
                        revision_hash(text.as_bytes()),
                    )
                }
            } else {
                (fallback_title, fallback_preview, fallback_hash)
            };
            let locator = note_locator(&path, block_id.as_deref());
            let open_conflict_count = self.open_conflicts_for(&path, &locator);
            notes.push(NoteSummaryDto {
                id,
                path,
                locator,
                block_id,
                captured_at,
                title,
                body_preview,
                content_hash,
                is_capture: true,
                organize_status,
                ov_resource_status,
                ov_memory_status,
                open_conflict_count,
            });
        }
        Ok(notes)
    }

    fn open_conflicts_for(&self, path: &str, locator: &str) -> usize {
        self.connection
            .query_row(
                "SELECT count(*) FROM conflicts
                 WHERE status = 'open'
                   AND (current_path IN (?1, ?2) OR incoming_path IN (?1, ?2))",
                params![path, locator],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
    }

    pub(crate) fn get_note_detail(&self, id_or_path: &str) -> Result<NoteDetailDto, StorageError> {
        let read_error = |error| database_error("note_detail", None, error);
        let source_error = |e: io::Error| {
            error(
                "SOURCE_UNAVAILABLE",
                "note_detail",
                None,
                true,
                "找不到原文，请检查文件路径、权限或占用情况",
                e,
            )
        };
        let (file_path, marker) = id_or_path
            .split_once(NOTE_ANCHOR)
            .map_or((id_or_path, None), |(p, m)| (p, Some(m)));

        let row: Option<(
            String,
            String,
            String,
            Option<String>,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
            String,
            String,
        )> = self
            .connection
            .query_row(
                "SELECT id, source_id, relative_path, block_id, captured_at, organize_status, organize_json,
                        ov_resource_status, ov_resource_uri, ov_memory_status, draft_id
                 FROM notes
                 WHERE id = ?1 OR (relative_path = ?2 AND block_id IS ?3)
                 ORDER BY rowid DESC LIMIT 1",
                params![id_or_path, file_path, marker],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                    ))
                },
            )
            .optional()
            .map_err(read_error)?;

        let (
            id,
            source_id,
            path,
            block_id,
            captured_at,
            organize_status,
            organize_json,
            ov_resource_status,
            ov_resource_uri,
            ov_memory_status,
            is_capture,
            raw_bytes,
        ) = if let Some(found) = row {
            let locator = note_locator(&found.2, found.3.as_deref());
            let raw_bytes =
                read_source_bytes(&self.workspace.files, &locator).map_err(source_error)?;
            (
                found.0,
                found.1,
                found.2,
                found.3,
                Some(found.4),
                found.5,
                found.6,
                found.7,
                found.8,
                found.9,
                found.10 != "manual",
                raw_bytes,
            )
        } else {
            if marker.is_some() {
                return Err(error(
                    "SOURCE_UNAVAILABLE",
                    "note_detail",
                    None,
                    false,
                    "找不到对应的随手记区块",
                    id_or_path,
                ));
            }
            let (key, full_path) = self
                .workspace
                .files
                .markdown_file(file_path)
                .map_err(source_error)?;
            let raw_bytes = fs::read(full_path).map_err(source_error)?;
            let captured_at = self
                .connection
                .query_row(
                    "SELECT captured_at FROM search_documents WHERE relative_path = ?1",
                    [&key],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()
                .map_err(read_error)?
                .flatten();
            (
                format!("file:{key}"),
                String::new(),
                key,
                None,
                captured_at,
                "idle".to_owned(),
                None,
                "pending".to_owned(),
                None,
                "pending".to_owned(),
                false,
                raw_bytes,
            )
        };

        let locator = note_locator(&path, block_id.as_deref());
        let text = String::from_utf8(raw_bytes).map_err(|e| {
            error(
                "SOURCE_UNAVAILABLE",
                "note_detail",
                None,
                false,
                "原文不是有效的 UTF-8 Markdown",
                e,
            )
        })?;
        let (body, content_hash) = if block_id.is_some() {
            let hash = revision_hash(text.as_bytes());
            (text, hash)
        } else {
            let hash = revision_hash(text.as_bytes());
            let body = if !source_id.is_empty() {
                text.strip_prefix(&capture_header(&id, &source_id))
                    .unwrap_or(&text)
                    .to_owned()
            } else {
                text
            };
            (body, hash)
        };
        let organize = organize_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<OrganizeResultDto>(raw).ok());
        let organize_error = self
            .connection
            .query_row(
                "SELECT last_error FROM background_jobs
                 WHERE note_id = ?1 AND kind = 'organize' AND status = 'failed'
                 ORDER BY rowid DESC LIMIT 1",
                [&id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(read_error)?
            .flatten()
            .or_else(|| organize.as_ref().and_then(|o| o.last_error.clone()));

        let conflicts = self.conflicts_for_paths(&path, &locator)?;
        let wiki_suggestions = if is_capture {
            self.wiki_suggestions_for_note(&id)?
        } else {
            self.wiki_suggestions_for_target(&path)?
        };
        let ov_error = self.memory_sync_error(
            &id,
            &path,
            ov_resource_status == "failed",
            ov_memory_status == "failed",
        )?;
        Ok(NoteDetailDto {
            id,
            path,
            locator,
            block_id,
            captured_at,
            title: note_title(&body),
            body,
            content_hash,
            is_capture,
            organize_status,
            organize,
            organize_error,
            ov_resource_status,
            ov_resource_uri,
            ov_memory_status,
            ov_error,
            conflicts,
            wiki_suggestions,
        })
    }

    pub(crate) fn update_note(
        &mut self,
        id_or_path: &str,
        expected_hash: &str,
        body: &str,
    ) -> Result<(String, Option<String>), StorageError> {
        let db_error = |e| database_error("update_note", None, e);
        let source_error = |e: io::Error| {
            error(
                "SOURCE_UNAVAILABLE",
                "update_note",
                None,
                true,
                "无法读取原文，请检查文件是否可用",
                e,
            )
        };
        let conflict = || {
            error(
                "VERSION_CONFLICT",
                "update_note",
                None,
                true,
                "笔记原文已在外部被修改，请刷新或复制当前内容后重试",
                "Markdown changed on disk",
            )
        };
        let (file_path, marker) = id_or_path
            .split_once(NOTE_ANCHOR)
            .map_or((id_or_path, None), |(p, m)| (p, Some(m)));
        let record = self
            .connection
            .query_row(
                "SELECT id, source_id, relative_path, block_id, captured_at, organize_json, draft_id
                 FROM notes
                 WHERE id = ?1 OR (relative_path = ?2 AND block_id IS ?3)
                 ORDER BY rowid DESC LIMIT 1",
                params![id_or_path, file_path, marker],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?;
        let is_capture = record
            .as_ref()
            .is_some_and(|(_, _, _, _, _, _, draft_id)| draft_id != "manual");
        if is_capture && body.trim().is_empty() {
            return Err(error(
                "INVALID_INPUT",
                "update_note",
                None,
                false,
                "笔记正文不能为空",
                "capture body is empty",
            ));
        }
        let missing_block = || {
            error(
                "SOURCE_UNAVAILABLE",
                "update_note",
                None,
                true,
                "找不到对应的随手记区块，文件可能已被外部改写",
                id_or_path,
            )
        };
        let (note_id, source_id, path, block_id, captured_at, organize_json) = match record {
            Some((id, source, path, block, captured, organize, _)) => {
                (Some(id), source, path, block, Some(captured), organize)
            }
            None if marker.is_some() => return Err(missing_block()),
            None => (
                None,
                String::new(),
                self.workspace
                    .files
                    .markdown_key(file_path)
                    .map_err(source_error)?,
                None,
                None,
                None,
            ),
        };
        let (key, full_path) = self
            .workspace
            .files
            .markdown_file(&path)
            .map_err(source_error)?;
        wikra::ensure_editable(&self.connection, &key, "update_note")?;
        let before_bytes = fs::read(&full_path).map_err(source_error)?;
        let before_text = std::str::from_utf8(&before_bytes).map_err(|e| {
            error(
                "SOURCE_UNAVAILABLE",
                "update_note",
                None,
                false,
                "原文不是有效的 UTF-8 Markdown",
                e,
            )
        })?;
        let lb = if newline(&before_bytes) == "crlf" {
            "\r\n"
        } else {
            "\n"
        };
        let (after_text, content_hash, body_for_meta) = if let Some(marker) = block_id.as_deref() {
            let (body_start, next_start, current_body) =
                block_span(before_text, marker).ok_or_else(missing_block)?;
            if revision_hash(current_body.as_bytes()) != expected_hash {
                return Err(conflict());
            }
            let normalized_body = body
                .trim_end_matches(['\r', '\n'])
                .replace("\r\n", "\n")
                .replace('\n', lb);
            let mut after = before_text[..body_start].to_owned();
            if !after.ends_with('\n') {
                after.push_str(lb);
            }
            if !after.ends_with("\n\n") && !after.ends_with("\n\r\n") {
                after.push_str(lb);
            }
            after.push_str(&normalized_body);
            after.push_str(lb);
            if next_start < before_text.len() {
                after.push_str(lb);
                after.push_str(&before_text[next_start..]);
            }
            (
                after,
                revision_hash(normalized_body.as_bytes()),
                normalized_body,
            )
        } else {
            if revision_hash(&before_bytes) != expected_hash {
                return Err(conflict());
            }
            let normalized_body = body.replace("\r\n", "\n").replace('\n', lb);
            let mut after = note_id
                .as_deref()
                .filter(|_| !source_id.is_empty())
                .map(|id| capture_header(id, &source_id))
                .filter(|header| before_text.starts_with(header))
                .unwrap_or_default();
            after.push_str(&normalized_body);
            if !after.is_empty() && !after.ends_with('\n') {
                after.push_str(lb);
            }
            let hash = revision_hash(after.as_bytes());
            (after, hash, normalized_body)
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        if let Some(id) = note_id.as_deref() {
            let organize_json = if let Some(mut result) = organize_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<OrganizeResultDto>(raw).ok())
            {
                result.source_changed = true;
                Some(serde_json::to_string(&result).map_err(|e| {
                    error(
                        "WRITE_FAILED",
                        "update_note",
                        Some(id),
                        true,
                        "无法更新整理结果状态",
                        e,
                    )
                })?)
            } else {
                organize_json
            };
            tx.execute(
                "UPDATE notes SET title = ?2, body_preview = ?3, content_hash = ?4, body_hash = ?5,
                                  organize_json = ?6, ov_memory_status = 'pending',
                                  updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
                params![
                    id,
                    note_title(&body_for_meta),
                    note_preview(&body_for_meta),
                    content_hash,
                    revision_hash(body.as_bytes()),
                    organize_json
                ],
            )
            .map_err(db_error)?;
            tx.execute(
                "UPDATE notes SET ov_resource_status = 'pending' WHERE relative_path = ?1",
                [&path],
            )
            .map_err(db_error)?;
        }
        let indexed_at = tx
            .query_row(
                "SELECT captured_at FROM search_documents WHERE relative_path = ?1",
                [&path],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(db_error)?;
        if indexed_at.is_some() || !is_capture {
            let captured_at = captured_at.or_else(|| indexed_at.flatten());
            let file_hash = if block_id.is_some() {
                revision_hash(after_text.as_bytes())
            } else {
                content_hash
            };
            search::upsert_document(&tx, &path, captured_at.as_deref(), &file_hash, &after_text)
                .map_err(db_error)?;
        }
        replace_markdown(&full_path, Some(&before_bytes), after_text.as_bytes()).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                conflict()
            } else {
                error(
                    "WRITE_FAILED",
                    "update_note",
                    None,
                    true,
                    "笔记原文写入失败",
                    e,
                )
            }
        })?;
        tx.commit().map_err(db_error)?;
        Ok((note_locator(&path, block_id.as_deref()), note_id))
    }

    pub(crate) fn ensure_note_for_source(
        &mut self,
        id_or_path: &str,
    ) -> Result<String, StorageError> {
        let db_error = |e| database_error("note", None, e);
        let file_path = physical_path(id_or_path);
        let marker = id_or_path.split_once(NOTE_ANCHOR).map(|(_, m)| m);
        if let Some(existing) = self
            .connection
            .query_row(
                "SELECT id FROM notes
                 WHERE id = ?1 OR (relative_path = ?2 AND block_id IS ?3)
                 ORDER BY rowid DESC LIMIT 1",
                params![id_or_path, file_path, marker],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(db_error)?
        {
            return Ok(existing);
        }
        let key = self.workspace.files.markdown_key(file_path).map_err(|e| {
            error(
                "SOURCE_UNAVAILABLE",
                "note",
                None,
                false,
                "找不到可整理的 Markdown 文件",
                e,
            )
        })?;
        let bytes = self.workspace.files.read_markdown(&key).map_err(|e| {
            error(
                "SOURCE_UNAVAILABLE",
                "note",
                None,
                true,
                "无法读取 Markdown 文件",
                e,
            )
        })?;
        let text = std::str::from_utf8(&bytes).map_err(|e| {
            error(
                "INVALID_INPUT",
                "note",
                None,
                false,
                "文件不是有效的 UTF-8 Markdown",
                e,
            )
        })?;
        let note_id: String = self
            .connection
            .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))
            .map_err(db_error)?;
        let now: String = self
            .connection
            .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
                r.get(0)
            })
            .map_err(db_error)?;
        let hash = revision_hash(&bytes);
        let request_id = format!("manual-{note_id}");
        self.connection
            .execute(
                "INSERT INTO notes (id, request_id, source_id, relative_path, block_id, captured_at,
                    title, body_preview, content_hash, organize_status, draft_id, draft_revision, body_hash, saved_at, updated_at)
                 VALUES (?1, ?2, ?1, ?3, NULL, ?4, ?5, ?6, ?7, 'idle', 'manual', 0, ?7, ?4, ?4)",
                params![
                    note_id,
                    request_id,
                    key,
                    now,
                    note_title(text),
                    note_preview(text),
                    hash
                ],
            )
            .map_err(db_error)?;
        Ok(note_id)
    }

    pub fn save_record(
        &mut self,
        request_id: &str,
        draft_id: &str,
        draft_revision: i64,
        body: &str,
    ) -> Result<SavedRecord, StorageError> {
        self.save_record_with_buffer(request_id, draft_id, draft_revision, body, None)
    }

    pub(crate) fn save_record_with_buffer(
        &mut self,
        request_id: &str,
        draft_id: &str,
        draft_revision: i64,
        body: &str,
        mode: Option<wikra::organize::CaptureMode>,
    ) -> Result<SavedRecord, StorageError> {
        if request_id.is_empty()
            || draft_id.is_empty()
            || draft_revision < 0
            || body.trim().is_empty()
        {
            return Err(error(
                "INVALID_INPUT",
                "capture",
                Some(request_id),
                false,
                "保存请求或正文无效",
                "invalid capture input",
            ));
        }
        let db_error = |e| database_error("capture", Some(request_id), e);
        let file_error = |e: io::Error| {
            let changed = e.kind() == io::ErrorKind::AlreadyExists;
            error(
                if changed {
                    "VERSION_CONFLICT"
                } else {
                    "WRITE_FAILED"
                },
                "capture",
                Some(request_id),
                true,
                if changed {
                    "每日文件已有新版本，请重试"
                } else {
                    "每日文件写入失败"
                },
                e,
            )
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let body_hash = revision_hash(body.as_bytes());
        let existing = tx.query_row(
            "SELECT id, source_id, content_hash, relative_path, saved_at, draft_id, draft_revision, body_hash
             FROM notes WHERE request_id = ?1", [request_id], |row| {
                Ok((SavedRecord {
                    record_id: row.get(0)?, source_id: row.get(1)?, source_revision: row.get(2)?,
                    relative_path: row.get(3)?, saved_at: row.get(4)?,
                }, row.get::<_, String>(5)?, row.get::<_, i64>(6)?, row.get::<_, String>(7)?))
            },
        ).optional().map_err(db_error)?;
        if let Some((saved, id, revision, hash)) = existing {
            if id != draft_id || revision != draft_revision || hash != body_hash {
                return Err(error(
                    "VERSION_CONFLICT",
                    "capture",
                    Some(request_id),
                    false,
                    "同一保存请求的内容不一致",
                    "request ID reused with different input",
                ));
            }
            return Ok(saved);
        }
        let persisted: Option<(i64, Option<String>)> = tx
            .query_row(
                "SELECT persisted_revision, body FROM drafts WHERE id = ?1",
                [draft_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        if persisted
            .as_ref()
            .and_then(|(rev, text)| text.as_deref().map(|text| (*rev, text)))
            != Some((draft_revision, body))
        {
            if mode.is_some()
                && persisted
                    .as_ref()
                    .is_none_or(|(rev, _)| *rev < draft_revision)
            {
                tx.execute(
                    "INSERT INTO drafts (id, body, edit_revision, persisted_revision, updated_at)
                     VALUES (?1, ?2, ?3, ?3, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
                     ON CONFLICT(id) DO UPDATE SET body = excluded.body, edit_revision = excluded.edit_revision,
                     persisted_revision = excluded.persisted_revision, updated_at = excluded.updated_at",
                    params![draft_id, body, draft_revision],
                ).map_err(db_error)?;
            } else {
                return Err(error(
                    "VERSION_CONFLICT",
                    "capture",
                    Some(request_id),
                    false,
                    "请先保存当前草稿",
                    "persisted draft does not match capture",
                ));
            }
        }

        let (date, time, captured_at, saved_at): (String, String, String, String) = tx
            .query_row(
                "SELECT strftime('%Y-%m-%d', t, 'unixepoch', 'localtime'),
                    strftime('%H:%M:%S', t, 'unixepoch', 'localtime'),
                    strftime('%Y-%m-%dT%H:%M:%fZ', t, 'unixepoch'),
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM (SELECT coalesce(?1 / 1000.0, unixepoch('subsec')) AS t)",
                [request_time(request_id)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(db_error)?;
        let relative_path = format!("{}/{date}.md", self.workspace.capture_dir);
        let (relative_path, path) = self
            .workspace
            .files
            .markdown_target(&relative_path)
            .map_err(file_error)?;
        // A directory or file link can resolve to a daily file at the workspace root.
        if path.parent() == Some(self.workspace.files.root_path()) {
            return Err(file_error(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "capture directory must be a subfolder inside the workspace",
            )));
        }
        let before = match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(file_error(e)),
        };
        let id = revision_hash(request_id.as_bytes());
        let (record_id, marker) = (&id[..32], &id[..16]);
        let block = body.trim_end_matches(['\r', '\n']);
        let revision = revision_hash(block.as_bytes());
        let mut after = before.clone().unwrap_or_else(|| format!("# {date}\n"));
        let appended = match block_body(&after, marker) {
            Some(existing) if existing == block => false,
            Some(_) => {
                return Err(error(
                    "VERSION_CONFLICT",
                    "capture",
                    Some(request_id),
                    false,
                    "该条记录已写入且原文已修改",
                    "capture block changed on disk",
                ));
            }
            None => {
                let line_break = if newline(after.as_bytes()) == "crlf" {
                    "\r\n"
                } else {
                    "\n"
                };
                if !after.is_empty() {
                    if !after.ends_with(line_break) {
                        after.push_str(line_break);
                    }
                    if !after
                        .strip_suffix(line_break)
                        .is_some_and(|text| text.ends_with('\n'))
                    {
                        after.push_str(line_break);
                    }
                }
                after.push_str(&format!(
                    "## {time} <!-- fmd-note:{marker} -->{line_break}{line_break}{body}"
                ));
                after.push_str(line_break);
                true
            }
        };
        let title = note_title(block);
        let preview = note_preview(block);
        tx.execute(
            "INSERT INTO notes (id, request_id, source_id, relative_path, block_id, captured_at, title, body_preview,
                content_hash, organize_status, draft_id, draft_revision, body_hash, saved_at, updated_at)
             VALUES (?1, ?2, ?1, ?3, ?4, ?5, ?6, ?7, ?8, 'idle', ?9, ?10, ?11, ?12, ?12)",
            params![record_id, request_id, relative_path, marker, captured_at, title, preview,
                revision, draft_id, draft_revision, body_hash, saved_at],
        ).map_err(db_error)?;
        if let Some(mode) = mode {
            wikra::organize::schedule_capture(&tx, record_id, &saved_at, mode)?;
        }
        search::upsert_document(
            &tx,
            &relative_path,
            Some(&captured_at),
            &revision_hash(after.as_bytes()),
            &after,
        )
        .map_err(db_error)?;
        if appended {
            replace_markdown(
                &path,
                before.as_deref().map(str::as_bytes),
                after.as_bytes(),
            )
            .map_err(file_error)?;
        }
        tx.commit().map_err(db_error)?;
        Ok(SavedRecord {
            record_id: record_id.into(),
            source_id: record_id.into(),
            source_revision: revision,
            relative_path,
            saved_at,
        })
    }

    pub fn clear_committed_draft(&mut self, request_id: &str) -> Result<bool, StorageError> {
        let count = self
            .connection
            .execute(
                "UPDATE drafts SET body = NULL WHERE body IS NOT NULL AND EXISTS (
                SELECT 1 FROM notes n WHERE n.request_id = ?1 AND n.draft_id = drafts.id
                AND n.draft_revision = drafts.persisted_revision)",
                [request_id],
            )
            .map_err(|e| database_error("draft_confirmation", Some(request_id), e))?;
        Ok(count == 1)
    }
}

fn request_time(id: &str) -> Option<i64> {
    let (stamp, _) = id.split_once('-')?;
    if stamp.len() == 13 {
        stamp.parse().ok()
    } else {
        None
    }
}

pub(crate) fn physical_path(path: &str) -> &str {
    path.split_once(NOTE_ANCHOR).map_or(path, |(path, _)| path)
}

pub(crate) fn is_in_capture_dir(key: &str, capture_dir: &str) -> bool {
    key.strip_prefix(capture_dir)
        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
}

fn is_capture(conn: &Connection, path: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM notes WHERE relative_path = ?1 AND draft_id <> 'manual')",
        [physical_path(path)],
        |row| row.get(0),
    )
    .unwrap_or(false)
}

fn block_body<'a>(text: &'a str, id: &str) -> Option<&'a str> {
    block_span(text, id).map(|(_, _, body)| body)
}

fn block_body_with_start<'a>(text: &'a str, id: &str) -> Option<(usize, &'a str)> {
    block_span(text, id).map(|(start, _, body)| (start, body))
}

fn block_span<'a>(text: &'a str, id: &str) -> Option<(usize, usize, &'a str)> {
    let mut start = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let plain = line.trim_end_matches(['\r', '\n']);
        if let Some((time, marker)) = plain
            .strip_prefix("## ")
            .and_then(|line| line.split_once(" <!-- fmd-note:"))
            && time.len() == 8
            && let Some(marker) = marker.strip_suffix(" -->")
            && (8..=16).contains(&marker.len())
            && marker.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            if let Some(start) = start {
                return Some((
                    start,
                    offset,
                    text[start..offset].trim_end_matches(['\r', '\n']),
                ));
            }
            if marker == id {
                let content = offset + line.len();
                let remainder = &text[content..];
                start = Some(
                    content
                        + if remainder.starts_with("\r\n") {
                            2
                        } else {
                            usize::from(remainder.starts_with('\n'))
                        },
                );
            }
        }
        offset += line.len();
    }
    start.map(|start| {
        (
            start,
            text.len(),
            text[start..].trim_end_matches(['\r', '\n']),
        )
    })
}

fn read_source_bytes(files: &WorkspaceFiles, path: &str) -> io::Result<Vec<u8>> {
    if let Some((path, marker)) = path.split_once(NOTE_ANCHOR) {
        let text = String::from_utf8(files.read_markdown(path)?).map_err(io::Error::other)?;
        let body = block_body(&text, marker)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "capture block is missing"))?;
        Ok(body.as_bytes().to_vec())
    } else {
        files.read_markdown(path)
    }
}

fn capture_header(record_id: &str, source_id: &str) -> String {
    format!("---\nrecord_id: {record_id}\nsource_id: {source_id}\n---\n")
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

pub(crate) fn note_locator(path: &str, block_id: Option<&str>) -> String {
    match block_id {
        Some(id) if !id.is_empty() => format!("{path}{NOTE_ANCHOR}{id}"),
        _ => path.to_owned(),
    }
}

pub(crate) fn note_title(body: &str) -> String {
    body.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .trim_start_matches('#')
        .trim()
        .chars()
        .take(120)
        .collect()
}

pub(crate) fn note_preview(body: &str) -> String {
    body.trim().chars().take(180).collect()
}

fn revision_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) const APPLICATION_ID: i64 = i64::from_be_bytes(*b"\0\0\0\0FMD1");
pub(crate) const SCHEMA_VERSION: i64 = 8;
const SCHEMA: &str = "
CREATE TABLE workspace_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE drafts (id TEXT PRIMARY KEY, body TEXT, edit_revision INTEGER NOT NULL CHECK(edit_revision >= 0), persisted_revision INTEGER NOT NULL CHECK(persisted_revision = edit_revision), updated_at TEXT NOT NULL);";
const NOTES_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS notes (
    id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, source_id TEXT NOT NULL,
    relative_path TEXT NOT NULL, block_id TEXT, captured_at TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '', body_preview TEXT NOT NULL DEFAULT '', content_hash TEXT NOT NULL,
    organize_status TEXT NOT NULL DEFAULT 'idle', organize_json TEXT,
    ov_resource_status TEXT NOT NULL DEFAULT 'pending', ov_resource_uri TEXT,
    ov_memory_status TEXT NOT NULL DEFAULT 'pending', ov_task_id TEXT,
    draft_id TEXT NOT NULL, draft_revision INTEGER NOT NULL, body_hash TEXT NOT NULL,
    saved_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS notes_file_block ON notes(relative_path, block_id);
CREATE INDEX IF NOT EXISTS notes_recent ON notes(captured_at);
CREATE TABLE IF NOT EXISTS conversations (id TEXT PRIMARY KEY, title TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS chat_messages (
    id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role TEXT NOT NULL, content TEXT NOT NULL, citations_json TEXT, conflict_ids_json TEXT, created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS background_jobs (
    id TEXT PRIMARY KEY, kind TEXT NOT NULL, note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    payload_json TEXT, status TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT, updated_at TEXT NOT NULL
);";

fn migration_error(detail: rusqlite::Error) -> StorageError {
    database_error("migration", None, detail)
}

fn table_exists(tx: &Connection, name: &str) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get(0),
    )
}

fn column_exists(tx: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut stmt = tx.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn migrate_workspace(tx: &Connection, version: i64) -> Result<(), StorageError> {
    if version == 1 {
        tx.execute_batch(search::SCHEMA).map_err(migration_error)?;
    } else if version < 4 {
        tx.execute_batch(search::MIGRATE_V4)
            .map_err(migration_error)?;
    }
    if table_exists(tx, "wiki_changes").map_err(migration_error)? {
        tx.execute_batch("ALTER TABLE wiki_changes RENAME TO wiki_suggestions;")
            .map_err(migration_error)?;
    }
    if table_exists(tx, "m2_model").map_err(migration_error)? {
        tx.execute_batch(
            "INSERT OR IGNORE INTO workspace_settings (key, value)
             SELECT 'model_config', json_object('endpoint', endpoint, 'model', model,
                    'enabled', json(CASE WHEN enabled THEN 'true' ELSE 'false' END))
             FROM m2_model WHERE id = 1;
             DROP TABLE m2_model;",
        )
        .map_err(migration_error)?;
    }
    tx.execute_batch("DROP TABLE IF EXISTS m2_preparations;")
        .map_err(migration_error)?;
    m2_answer::migrate_conflicts(tx).map_err(migration_error)?;
    Ok(())
}

fn migrate_to_v6(tx: &Connection) -> Result<(), StorageError> {
    if table_exists(tx, "wiki_suggestions").map_err(migration_error)?
        && column_exists(tx, "wiki_suggestions", "phase").map_err(migration_error)?
    {
        tx.execute_batch(
            "CREATE TABLE wiki_suggestions_v6 (
                id TEXT PRIMARY KEY,
                note_id TEXT,
                target_path TEXT NOT NULL,
                base_hash TEXT NOT NULL,
                before_text TEXT NOT NULL,
                after_text TEXT NOT NULL,
                reason TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             INSERT OR IGNORE INTO wiki_suggestions_v6 (
                id, note_id, target_path, base_hash, before_text, after_text, reason, status, created_at
             )
             SELECT id,
                    (SELECT n.id FROM notes n
                     WHERE (n.relative_path || coalesce('#fmd-note:' || n.block_id, '')) = source_path
                        OR n.relative_path = source_path
                     LIMIT 1),
                    target,
                    base_hash,
                    before_text,
                    after_text,
                    coalesce(nullif(entry_text, ''), source_excerpt),
                    CASE phase WHEN 'deferred' THEN 'pending' WHEN 'prepared' THEN 'pending' ELSE phase END,
                    strftime('%Y-%m-%dT%H:%M:%fZ','now')
             FROM wiki_suggestions
             WHERE kind = 'wiki' AND phase <> 'superseded';
             DROP TABLE wiki_suggestions;
             ALTER TABLE wiki_suggestions_v6 RENAME TO wiki_suggestions;",
        )
        .map_err(migration_error)?;
    }
    tx.execute_batch(wiki::SCHEMA).map_err(migration_error)?;
    tx.execute_batch(
        "DROP TABLE IF EXISTS m2_analyses;
         DROP TABLE IF EXISTS m2_answers;
         DROP TABLE IF EXISTS jobs;
         DROP TABLE IF EXISTS source_revisions;
         DROP TABLE IF EXISTS sources;
         UPDATE notes SET organize_status = 'idle' WHERE organize_status = 'pending';",
    )
    .map_err(migration_error)?;
    Ok(())
}

fn migrate_captures(tx: &Connection) -> Result<(), StorageError> {
    if !table_exists(tx, "capture_intents").map_err(migration_error)? {
        return Ok(());
    }
    tx.execute_batch(
        "INSERT INTO notes (id, request_id, source_id, relative_path, captured_at, content_hash,
            draft_id, draft_revision, body_hash, saved_at, updated_at)
         SELECT r.id, r.capture_request_id, r.source_id, i.relative_path, r.captured_at, i.revision,
            i.draft_id, i.draft_revision, i.body_hash, i.saved_at, i.saved_at
         FROM records r JOIN capture_intents i ON r.capture_request_id = i.request_id
         WHERE i.state = 'committed' AND r.lifecycle = 'active' ORDER BY i.rowid;
         WITH unfinished AS (
            SELECT draft_id, draft_revision, body_hash,
                CAST(substr(file_bytes, length(CAST(
                    '---' || char(10) || 'record_id: ' || record_id || char(10)
                    || 'source_id: ' || source_id || char(10) || '---' || char(10) AS BLOB)) + 1) AS TEXT) AS body
            FROM capture_intents WHERE state <> 'committed' AND file_bytes IS NOT NULL ORDER BY rowid
         )
         INSERT INTO drafts (id, body, edit_revision, persisted_revision, updated_at)
         SELECT draft_id, group_concat(body, char(10,10)), max(draft_revision) + 1, max(draft_revision) + 1,
             strftime('%Y-%m-%dT%H:%M:%fZ','now')
         FROM unfinished u
         WHERE NOT EXISTS(SELECT 1 FROM drafts d WHERE d.id = u.draft_id AND d.body = u.body)
           AND NOT EXISTS(SELECT 1 FROM notes n WHERE n.draft_id = u.draft_id
                AND n.draft_revision = u.draft_revision AND n.body_hash = u.body_hash)
         GROUP BY draft_id
         ON CONFLICT(id) DO UPDATE SET
            body = CASE WHEN drafts.body IS NULL THEN excluded.body ELSE
                drafts.body || char(10,10) || '## 从旧版恢复的未完成记录' || char(10,10) || excluded.body END,
            edit_revision = max(drafts.edit_revision + 1, excluded.edit_revision),
            persisted_revision = max(drafts.edit_revision + 1, excluded.edit_revision),
            updated_at = excluded.updated_at;
         DROP TABLE records;
         DROP TABLE capture_intents;",
    )
    .map_err(migration_error)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
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
    fn empty_and_dot_capture_paths_create_and_write_to_notes() {
        for input in ["", "."] {
            let dir = TestDir::new();
            let mut storage = Storage::open(&dir.0, Path::new(input), ".flash-test").unwrap();
            assert_eq!(storage.capture_dir(), "notes");
            assert!(dir.0.join("notes").is_dir());

            storage.save_draft("draft", 1, "默认目录正文").unwrap();
            let saved = storage
                .save_record("default", "draft", 1, "默认目录正文")
                .unwrap();
            let daily_file = saved.relative_path.strip_prefix("notes/").unwrap();
            assert!(
                fs::read_to_string(dir.0.join(&saved.relative_path))
                    .unwrap()
                    .contains("默认目录正文")
            );
            assert!(!dir.0.join(daily_file).exists());
        }
    }

    #[test]
    fn missing_nested_capture_directory_is_created_before_saving() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("journal/daily"), ".flash-test").unwrap();
        assert_eq!(storage.capture_dir(), "journal/daily");
        assert!(dir.0.join("journal/daily").is_dir());

        storage.save_draft("draft", 1, "嵌套目录正文").unwrap();
        let saved = storage
            .save_record("nested", "draft", 1, "嵌套目录正文")
            .unwrap();
        let daily_file = saved.relative_path.strip_prefix("journal/daily/").unwrap();
        assert!(dir.0.join(&saved.relative_path).is_file());
        assert!(!dir.0.join(daily_file).exists());
    }

    #[test]
    fn daily_notes_append_personal_text_and_deduplicate_requests() {
        let dir = TestDir::new();
        let mut storage = dir.open();
        let today: String = storage
            .connection
            .query_row("SELECT date('now','localtime')", [], |row| row.get(0))
            .unwrap();
        let file = dir.0.join(format!("notes/{today}.md"));
        fs::write(&file, "手工记录\r\n保留原文").unwrap();

        storage
            .save_draft("draft", 1, "第一条记录\r\n保留换行")
            .unwrap();
        let first = storage
            .save_record("first", "draft", 1, "第一条记录\r\n保留换行")
            .unwrap();
        assert_eq!(
            storage
                .save_record("first", "draft", 1, "第一条记录\r\n保留换行")
                .unwrap(),
            first
        );
        assert_eq!(
            storage
                .save_record("first", "draft", 1, "不同正文")
                .unwrap_err()
                .code,
            "VERSION_CONFLICT"
        );
        storage.save_draft("draft", 2, "第二条记录").unwrap();
        let second = storage
            .save_record("second", "draft", 2, "第二条记录")
            .unwrap();
        assert_eq!(first.relative_path, second.relative_path);
        let disk = fs::read_to_string(&file).unwrap();
        assert!(disk.starts_with("手工记录\r\n保留原文"));
        assert_eq!(disk.matches("<!-- fmd-note:").count(), 2);

        let captures = storage.list_recent_captures().unwrap();
        assert_eq!(
            (captures[0].2.as_str(), captures[1].2.as_str()),
            ("第二条记录", "第一条记录\r\n保留换行")
        );
        let first_locator = format!(
            "{}{NOTE_ANCHOR}{}",
            first.relative_path,
            captures[1].4.as_deref().unwrap()
        );
        let mut search = storage.search_reader().unwrap();
        assert_eq!(
            search
                .read_record_source(&first_locator, &first.source_revision)
                .unwrap()
                .2,
            "第一条记录\r\n保留换行"
        );
        assert_eq!(
            search
                .search_records("第二条记录", None, None, 0)
                .unwrap()
                .0[0]
                .0,
            second.relative_path
        );
        assert!(!storage.clear_committed_draft("first").unwrap());
        assert!(storage.clear_committed_draft("second").unwrap());
        assert!(storage.load_draft("draft").unwrap().is_none());
    }

    #[test]
    fn update_note_preserves_adjacent_daily_blocks_and_line_endings() {
        for lb in ["\n", "\r\n"] {
            let dir = TestDir::new();
            let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
            let original_bodies = ["首条保持原样", "中间旧正文", "末条旧正文"];
            let records = original_bodies.map(|body| {
                storage.save_draft(body, 1, body).unwrap();
                storage
                    .save_record(&format!("1795579200000-{body}"), body, 1, body)
                    .unwrap()
            });
            assert!(
                records
                    .iter()
                    .all(|note| note.relative_path == records[0].relative_path)
            );
            let file = dir.0.join(&records[0].relative_path);
            let original = fs::read_to_string(&file).unwrap().replace('\n', lb);
            let date = file.file_stem().unwrap().to_str().unwrap();
            assert!(original.starts_with(&format!("# {date}{lb}{lb}")));
            fs::write(&file, &original).unwrap();

            let middle = storage.get_note_detail(&records[1].record_id).unwrap();
            let new_middle = "# 中间新标题\n第二行\r\n\n";
            let (middle_locator, note_id) = storage
                .update_note(&middle.id, &middle.content_hash, new_middle)
                .unwrap();
            assert_eq!(middle_locator, middle.locator);
            assert_eq!(note_id.as_deref(), Some(middle.id.as_str()));
            let middle_body = format!("# 中间新标题{lb}第二行");
            let edited = original.replace(original_bodies[1], &middle_body);
            assert_eq!(fs::read_to_string(&file).unwrap(), edited);

            let tail = storage.get_note_detail(&records[2].record_id).unwrap();
            storage
                .update_note(&tail.locator, &tail.content_hash, "末条新正文\r\n")
                .unwrap();
            let disk = fs::read_to_string(&file).unwrap();
            assert_eq!(disk, edited.replace(original_bodies[2], "末条新正文"));
            assert_eq!(
                storage.get_note_detail(&records[0].record_id).unwrap().body,
                original_bodies[0]
            );
            let refreshed = storage.get_note_detail(&middle_locator).unwrap();
            assert_eq!(refreshed.body, middle_body);
            let metadata: (String, String, String, String) = storage
                .connection
                .query_row(
                    "SELECT title, body_preview, content_hash, body_hash FROM notes WHERE id = ?1",
                    [&middle.id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();
            assert_eq!(
                metadata,
                (
                    "中间新标题".into(),
                    middle_body,
                    refreshed.content_hash,
                    revision_hash(new_middle.as_bytes())
                )
            );

            let (start, _, _) = block_span(&disk, tail.block_id.as_deref().unwrap()).unwrap();
            let header_only = disk[..start].trim_end_matches(['\r', '\n']);
            fs::write(&file, header_only).unwrap();
            let empty = storage.get_note_detail(&tail.locator).unwrap();
            storage
                .update_note(&tail.locator, &empty.content_hash, "\n补回正文")
                .unwrap();
            assert_eq!(
                fs::read_to_string(&file).unwrap(),
                format!("{header_only}{lb}{lb}{lb}补回正文{lb}")
            );
            assert_eq!(
                storage.get_note_detail(&tail.locator).unwrap().body,
                format!("{lb}补回正文")
            );
        }
    }

    #[test]
    fn update_note_rejects_empty_captures_and_preserves_external_edits() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        let original = "随手记原文";
        storage.save_draft("draft", 1, original).unwrap();
        let saved = storage
            .save_record("edit-conflict", "draft", 1, original)
            .unwrap();
        let detail = storage.get_note_detail(&saved.record_id).unwrap();
        let file = dir.0.join(&saved.relative_path);
        let before = fs::read_to_string(&file).unwrap();
        let err = storage
            .update_note(&detail.locator, &detail.content_hash, " \r\n")
            .unwrap_err();
        assert_eq!(
            (err.code, err.safe_message),
            ("INVALID_INPUT", "笔记正文不能为空")
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), before);

        let external = before.replace(original, "外部编辑的新版本");
        fs::write(&file, &external).unwrap();
        let err = storage
            .update_note(&detail.locator, &detail.content_hash, "尝试覆盖")
            .unwrap_err();
        assert_eq!(err.code, "VERSION_CONFLICT");
        assert!(err.retryable);
        assert_eq!(fs::read_to_string(&file).unwrap(), external);

        fs::write(&file, "# 保留手写文件\n").unwrap();
        let err = storage
            .update_note(&detail.locator, &detail.content_hash, "尝试覆盖")
            .unwrap_err();
        assert_eq!(err.code, "SOURCE_UNAVAILABLE");
        let missing = note_locator(&saved.relative_path, Some("deadbeef"));
        assert_eq!(
            storage
                .update_note(&missing, &detail.content_hash, "不会创建区块")
                .unwrap_err()
                .code,
            "SOURCE_UNAVAILABLE"
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "# 保留手写文件\n");
    }

    #[test]
    fn update_note_saves_whole_markdown_and_updates_search_immediately() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        let path = "手写文档.md";
        let file = dir.0.join(path);
        let original = "# 旧文档\r\noldkeyword\r\n";
        fs::write(&file, original).unwrap();
        let mut reader = storage.search_reader().unwrap();
        assert_eq!(
            reader
                .search_records("oldkeyword", None, None, 0)
                .unwrap()
                .0
                .len(),
            1
        );

        let detail = storage.get_note_detail(path).unwrap();
        let (locator, note_id) = storage
            .update_note(
                &detail.locator,
                &detail.content_hash,
                "# 新文档\nnewkeyword",
            )
            .unwrap();
        assert_eq!((locator.as_str(), note_id), (path, None));
        let expected = "# 新文档\r\nnewkeyword\r\n";
        assert_eq!(fs::read_to_string(&file).unwrap(), expected);
        let indexed: String = storage
            .connection
            .query_row(
                "SELECT body FROM search_documents WHERE relative_path = ?1",
                [path],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexed, expected);
        let updated = reader
            .search_records("newkeyword", None, None, 0)
            .unwrap()
            .0;
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].0, path);
        assert!(
            reader
                .search_records("oldkeyword", None, None, 0)
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(
            storage
                .update_note(path, &detail.content_hash, "旧基线")
                .unwrap_err()
                .code,
            "VERSION_CONFLICT"
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), expected);

        let note_id = storage.ensure_note_for_source(path).unwrap();
        let detail = storage.get_note_detail(&note_id).unwrap();
        assert!(!detail.is_capture);
        assert_eq!(
            storage.update_note(path, &detail.content_hash, "").unwrap(),
            (path.into(), Some(note_id))
        );
        assert!(fs::read(&file).unwrap().is_empty());

        let blank = "未索引的空文档.md";
        fs::write(dir.0.join(blank), "").unwrap();
        storage
            .update_note(blank, &revision_hash(b""), "# 只有标题")
            .unwrap();
        assert_eq!(storage.get_note_detail(blank).unwrap().body, "# 只有标题\n");
    }

    #[test]
    fn update_note_keeps_the_header_of_legacy_capture_files() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        storage.save_draft("draft", 1, "旧正文").unwrap();
        let saved = storage
            .save_record("legacy-capture", "draft", 1, "旧正文")
            .unwrap();
        let path = "legacy.md";
        let header = capture_header(&saved.record_id, &saved.source_id);
        fs::write(dir.0.join(path), format!("{header}旧正文\n")).unwrap();
        storage
            .connection
            .execute(
                "UPDATE notes SET relative_path = ?2, block_id = NULL WHERE id = ?1",
                params![saved.record_id, path],
            )
            .unwrap();
        let detail = storage.get_note_detail(&saved.record_id).unwrap();
        assert_eq!(detail.body, "旧正文\n");

        let (locator, note_id) = storage
            .update_note(&detail.id, &detail.content_hash, "新正文")
            .unwrap();
        assert_eq!(
            (locator.as_str(), note_id.as_deref()),
            (path, Some(saved.record_id.as_str()))
        );
        assert_eq!(
            fs::read_to_string(dir.0.join(path)).unwrap(),
            format!("{header}新正文\n")
        );
        assert_eq!(storage.get_note_detail(&locator).unwrap().body, "新正文\n");
    }

    #[test]
    fn failed_daily_write_preserves_the_draft_and_can_retry() {
        let dir = TestDir::new();
        let mut storage = dir.open();
        let today: String = storage
            .connection
            .query_row("SELECT date('now','localtime')", [], |row| row.get(0))
            .unwrap();
        let blocked = dir.0.join(format!("notes/{today}.md"));
        fs::create_dir(&blocked).unwrap();
        storage.save_draft("draft", 1, "待重试版本").unwrap();
        assert!(
            storage
                .save_record("retry", "draft", 1, "待重试版本")
                .is_err()
        );
        assert_eq!(
            storage.load_draft("draft").unwrap().unwrap().body,
            "待重试版本"
        );
        fs::remove_dir(blocked).unwrap();
        let result = storage
            .save_record("retry", "draft", 1, "待重试版本")
            .unwrap();
        assert_eq!(storage.list_recent_captures().unwrap().len(), 1);
        assert_eq!(
            storage.committed_draft("draft").unwrap(),
            Some(("retry".into(), result.relative_path))
        );
    }

    #[test]
    fn workspace_can_move_and_reopen_with_a_different_save_directory() {
        let dir = TestDir::new();
        let parent = dir.0.join("before");
        let root = parent.join("notes");
        fs::create_dir_all(root.join("journal")).unwrap();
        let mut storage = Storage::open(&root, Path::new("."), ".flash-test").unwrap();
        storage.save_draft("draft", 1, "迁移中").unwrap();
        let id = storage.workspace_id().to_owned();
        let _idle_workspace = storage.into_workspace();
        let moved = dir.0.join("after");
        fs::rename(&parent, &moved).unwrap();

        let storage =
            Storage::open(&moved.join("notes"), Path::new("journal"), ".flash-test").unwrap();
        assert_eq!(storage.workspace_id(), id);
        assert_eq!(storage.capture_dir(), "journal");
        assert_eq!(storage.load_draft("draft").unwrap().unwrap().body, "迁移中");
    }

    #[test]
    fn migrates_v4_capture_and_model_data_without_rewriting_markdown() {
        let dir = TestDir::new();
        let state = dir.0.join(".flash-test");
        fs::create_dir(&state).unwrap();
        let db = Connection::open(state.join("state.sqlite")).unwrap();
        db.execute_batch(SCHEMA).unwrap();
        db.execute_batch(search::SCHEMA).unwrap();
        db.pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        db.pragma_update(None, "user_version", 4).unwrap();
        db.execute_batch(
            "INSERT INTO workspace_settings VALUES ('id', 'old-workspace'), ('credential:model', 'reference-only');
             INSERT INTO drafts VALUES ('quick-entry', '未提交草稿', 2, 2, '2026-11-24T06:00:00.000Z');
             CREATE TABLE capture_intents (
                request_id TEXT PRIMARY KEY, draft_id TEXT, draft_revision INTEGER, body_hash TEXT,
                relative_path TEXT, revision TEXT, state TEXT, saved_at TEXT,
                record_id TEXT, source_id TEXT, file_bytes BLOB
             );
             CREATE TABLE records (id TEXT, capture_request_id TEXT REFERENCES capture_intents(request_id),
                source_id TEXT, captured_at TEXT, lifecycle TEXT);
             CREATE TABLE m2_model (id INTEGER PRIMARY KEY, endpoint TEXT, model TEXT, enabled INTEGER);
             INSERT INTO m2_model VALUES (1, 'https://example.test/v1/chat/completions', 'fixture', 1);
             CREATE TABLE m2_preparations (id TEXT, data TEXT);
             CREATE TABLE disagreements (id TEXT PRIMARY KEY, question TEXT, relation TEXT, status TEXT, judgment TEXT, reason TEXT);
             CREATE TABLE disagreement_members (disagreement_id TEXT, member_order INTEGER, claim_id TEXT,
                member_text TEXT, evidence_json TEXT);
             CREATE TABLE disagreement_history (id INTEGER PRIMARY KEY, disagreement_id TEXT,
                recorded_at TEXT, actor TEXT, status TEXT, judgment TEXT, reason TEXT);
             INSERT INTO disagreements VALUES ('old-conflict','旧分歧','historical','decided','我选择保留','来自两条记录');
             INSERT INTO disagreement_members VALUES ('old-conflict',0,'a','旧观点A','{\"id\":\"a\",\"path\":\"old.md\",\"revision\":\"old\"}'),
                                                       ('old-conflict',1,'b','旧观点B','{\"id\":\"b\",\"path\":\"old.md\",\"revision\":\"old\"}');
             INSERT INTO disagreement_history VALUES (1,'old-conflict','2026-11-24T06:00:00.000Z','user','decided','我选择保留','来自两条记录');",
        ).unwrap();
        let body = "一记一文档旧记录";
        let original = format!("{}{body}", capture_header("record", "source"));
        let revision = revision_hash(original.as_bytes());
        let date = "2026-11-24T05:00:00.000Z";
        fs::write(dir.0.join("old.md"), &original).unwrap();
        db.execute("INSERT INTO capture_intents VALUES ('old-request','quick-entry',1,?1,'old.md',?2,'committed',?3,'record','source',NULL)",
            params![revision_hash(body.as_bytes()), revision, date]).unwrap();
        db.execute("INSERT INTO capture_intents VALUES ('old-pending','quick-entry',1,'pending','pending.md','old','failed',NULL,'pending-record','pending-source',?1)",
            [format!("{}失败前只存在于旧保存请求的草稿", capture_header("pending-record", "pending-source")).into_bytes()]).unwrap();
        db.execute(
            "INSERT INTO records VALUES ('record','old-request','source',?1,'active')",
            [date],
        )
        .unwrap();
        db.execute_batch(
            "CREATE TABLE sources (id TEXT PRIMARY KEY, location TEXT NOT NULL UNIQUE, current_revision TEXT NOT NULL, availability TEXT NOT NULL);
             CREATE TABLE source_revisions (source_id TEXT NOT NULL, revision TEXT NOT NULL, observed_at TEXT NOT NULL, snapshot BLOB NOT NULL, encoding TEXT NOT NULL, newline TEXT NOT NULL);",
        )
        .unwrap();
        db.execute(
            "INSERT INTO sources VALUES ('source','old.md',?1,'available')",
            [&revision],
        )
        .unwrap();
        db.execute(
            "INSERT INTO source_revisions VALUES ('source',?1,?2,?3,'utf-8','lf')",
            params![revision, date, original.as_bytes()],
        )
        .unwrap();
        search::upsert_document(&db, "old.md", Some(date), &revision, body).unwrap();
        db.execute("INSERT INTO search_status VALUES (1,1,0)", [])
            .unwrap();
        db.execute_batch(
            "CREATE TABLE wiki_targets (path TEXT PRIMARY KEY, kind TEXT NOT NULL CHECK(kind IN ('wiki','tasks')));
             CREATE TABLE wiki_changes (
                id TEXT PRIMARY KEY, operation_key TEXT NOT NULL, target TEXT NOT NULL, kind TEXT NOT NULL,
                phase TEXT NOT NULL, base_hash TEXT NOT NULL, after_hash TEXT NOT NULL, before_text TEXT NOT NULL,
                after_text TEXT NOT NULL, source_path TEXT NOT NULL, source_revision TEXT NOT NULL,
                start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, source_excerpt TEXT NOT NULL, entry_text TEXT NOT NULL,
                task_id TEXT, dependency TEXT, undoes TEXT
             );
             INSERT INTO wiki_changes VALUES ('w1','k1','Wiki.md','wiki','deferred','h1','h2','before','after','old.md','rev',1,1,'原文','同步决策',NULL,NULL,NULL);
             INSERT INTO wiki_targets VALUES ('Wiki.md','wiki');",
        )
        .unwrap();
        drop(db);

        let mut storage = dir.open();
        assert_eq!(
            storage
                .connection
                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
        for dropped in [
            "capture_intents",
            "m2_preparations",
            "disagreements",
            "sources",
            "source_revisions",
            "jobs",
            "m2_analyses",
            "m2_answers",
        ] {
            assert!(
                !table_exists(&storage.connection, dropped).unwrap(),
                "expected legacy table `{dropped}` to be removed"
            );
        }
        let migrated_wiki = storage.list_wiki_suggestions().unwrap();
        assert_eq!(migrated_wiki.len(), 1);
        assert_eq!(migrated_wiki[0].status, "pending");
        assert_eq!(migrated_wiki[0].reason, "同步决策");
        let legacy: String = storage
            .connection
            .query_row(
                "SELECT legacy_json FROM conflicts WHERE id='old-conflict'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            legacy.contains("我选择保留")
                && legacy.contains("旧观点B")
                && legacy.contains("2026-11-24T06:00:00.000Z")
        );
        assert_eq!(storage.workspace_id(), "old-workspace");
        assert_eq!(
            storage.wiki_targets().unwrap(),
            [("wiki".into(), "Wiki.md".into())]
        );
        assert_eq!(
            storage.credential_reference("model").unwrap().as_deref(),
            Some("reference-only")
        );
        assert_eq!(storage.model_config().unwrap().unwrap().1, "fixture");
        assert_eq!(
            storage.load_draft("quick-entry").unwrap().unwrap().body,
            "未提交草稿\n\n## 从旧版恢复的未完成记录\n\n失败前只存在于旧保存请求的草稿"
        );
        assert_eq!(
            storage.list_recent_captures().unwrap(),
            [(
                "old.md".into(),
                date.into(),
                body.into(),
                revision.clone(),
                None
            )]
        );
        assert_eq!(
            storage
                .save_record("old-request", "quick-entry", 1, body)
                .unwrap()
                .source_revision,
            revision
        );
        let mut search = storage.search_reader().unwrap();
        assert_eq!(
            search
                .search_records("一记一文档", None, None, 0)
                .unwrap()
                .0[0]
                .0,
            "old.md"
        );
        assert_eq!(fs::read_to_string(dir.0.join("old.md")).unwrap(), original);
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

    #[test]
    fn detects_unc_paths_for_rollback_journal_mode() {
        assert!(is_unc_path(Path::new(r"\\server\share\notes\state.sqlite")));
        assert!(is_unc_path(Path::new(
            r"\\?\UNC\server\share\notes\state.sqlite"
        )));
        assert!(!is_unc_path(Path::new(r"D:\notes\state.sqlite")));
        assert!(!is_unc_path(Path::new(r"\\?\D:\notes\state.sqlite")));

        let dir = TestDir::new();
        let storage = dir.open();
        let mode: String = storage
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_ascii_lowercase(), "wal");
    }
}
