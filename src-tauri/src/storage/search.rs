use std::collections::HashMap;
use std::io::ErrorKind;

use super::*;

pub(crate) type SearchHit = (String, Option<String>, String, String, String);
pub(crate) type SearchPage = (Vec<SearchHit>, bool, usize);

const PAGE_SIZE: usize = 50;
const EXCERPT_CHARS: i64 = 180;
const LEADING_CONTEXT_CHARS: i64 = 32;
const TITLE_CHARS: i64 = 120;

pub(super) const SCHEMA: &str = "
    CREATE TABLE search_documents (
        relative_path TEXT PRIMARY KEY,
        captured_at TEXT,
        revision TEXT NOT NULL,
        body TEXT NOT NULL,
        mtime_ns INTEGER NOT NULL DEFAULT 0,
        file_size INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE search_status (
        id INTEGER PRIMARY KEY CHECK (id = 1), generation INTEGER NOT NULL, skipped INTEGER NOT NULL
    );
    CREATE VIRTUAL TABLE search_documents_fts USING fts5(
        relative_path,
        body,
        tokenize='trigram',
        content='search_documents',
        content_rowid='rowid'
    );
    CREATE TRIGGER search_documents_ai AFTER INSERT ON search_documents BEGIN
        INSERT INTO search_documents_fts(rowid, relative_path, body)
        VALUES (new.rowid, new.relative_path, new.body);
    END;
    CREATE TRIGGER search_documents_ad AFTER DELETE ON search_documents BEGIN
        INSERT INTO search_documents_fts(search_documents_fts, rowid, relative_path, body)
        VALUES ('delete', old.rowid, old.relative_path, old.body);
    END;
    CREATE TRIGGER search_documents_au AFTER UPDATE OF relative_path, body ON search_documents BEGIN
        INSERT INTO search_documents_fts(search_documents_fts, rowid, relative_path, body)
        VALUES ('delete', old.rowid, old.relative_path, old.body);
        INSERT INTO search_documents_fts(rowid, relative_path, body)
        VALUES (new.rowid, new.relative_path, new.body);
    END;";

pub(super) const MIGRATE_V4: &str = "
    ALTER TABLE search_documents ADD COLUMN mtime_ns INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE search_documents ADD COLUMN file_size INTEGER NOT NULL DEFAULT 0;
    CREATE VIRTUAL TABLE IF NOT EXISTS search_documents_fts USING fts5(
        relative_path,
        body,
        tokenize='trigram',
        content='search_documents',
        content_rowid='rowid'
    );
    CREATE TRIGGER IF NOT EXISTS search_documents_ai AFTER INSERT ON search_documents BEGIN
        INSERT INTO search_documents_fts(rowid, relative_path, body)
        VALUES (new.rowid, new.relative_path, new.body);
    END;
    CREATE TRIGGER IF NOT EXISTS search_documents_ad AFTER DELETE ON search_documents BEGIN
        INSERT INTO search_documents_fts(search_documents_fts, rowid, relative_path, body)
        VALUES ('delete', old.rowid, old.relative_path, old.body);
    END;
    CREATE TRIGGER IF NOT EXISTS search_documents_au AFTER UPDATE OF relative_path, body ON search_documents BEGIN
        INSERT INTO search_documents_fts(search_documents_fts, rowid, relative_path, body)
        VALUES ('delete', old.rowid, old.relative_path, old.body);
        INSERT INTO search_documents_fts(rowid, relative_path, body)
        VALUES (new.rowid, new.relative_path, new.body);
    END;
    INSERT INTO search_documents_fts(search_documents_fts) VALUES('rebuild');";

pub(super) fn fts_trigram_phrase(term: &str) -> Option<String> {
    if term.chars().count() < 3 || term.contains('\0') {
        return None;
    }
    Some(format!("\"{}\"", term.replace('"', "\"\"")))
}

struct CachedDocMeta {
    captured_at: Option<String>,
    mtime_ns: i64,
    file_size: i64,
}

fn racy_cutoff_filetime() -> i64 {
    const EPOCH_DIFF_TICKS: u64 = 116_444_736_000_000_000;
    const RACY_WINDOW_TICKS: u64 = 20_000_000;
    let since_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let now_ticks = EPOCH_DIFF_TICKS
        .saturating_add(since_unix.as_secs().saturating_mul(10_000_000))
        .saturating_add((since_unix.subsec_nanos() / 100) as u64);
    now_ticks
        .saturating_sub(RACY_WINDOW_TICKS)
        .min(i64::MAX as u64) as i64
}

fn index_error(detail: rusqlite::Error) -> StorageError {
    database_error("search_index", None, detail)
}

pub(crate) struct SearchReader {
    connection: Connection,
    files: Arc<WorkspaceFiles>,
    build: Arc<Mutex<()>>,
}

impl Storage {
    #[cfg(test)]
    pub(crate) fn search_reader(&self) -> Result<SearchReader, StorageError> {
        self.workspace.search_reader()
    }
}

impl WorkspaceStorage {
    pub(crate) fn search_reader(&self) -> Result<SearchReader, StorageError> {
        Ok(SearchReader {
            connection: self.connect()?,
            files: self.files.clone(),
            build: self.search_build.clone(),
        })
    }
}

impl SearchReader {
    pub(crate) fn is_capture_path(&self, relative_path: &str) -> bool {
        is_capture(&self.connection, relative_path)
    }

    pub(crate) fn ensure_index_fresh(&mut self) -> Result<(), StorageError> {
        if self.generation()?.is_none() {
            self.build_index(None)?;
        }
        Ok(())
    }

    fn generation(&self) -> Result<Option<i64>, StorageError> {
        self.connection
            .query_row(
                "SELECT generation FROM search_status WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(index_error)
    }

    pub(crate) fn rebuild_search_index(&mut self) -> Result<(usize, usize), StorageError> {
        self.build_index(self.generation()?)
    }

    fn build_index(&mut self, before: Option<i64>) -> Result<(usize, usize), StorageError> {
        let build = self.build.clone();
        let _guard = build.lock().map_err(|e| {
            error(
                "WRITE_FAILED",
                "search_index",
                None,
                true,
                "索引更新中断，请重启后重试",
                e,
            )
        })?;
        if self.generation()? == before {
            self.scan_workspace()?;
        }
        self.connection.query_row(
            "SELECT (SELECT count(*) FROM search_documents), skipped FROM search_status WHERE id = 1",
            [], |row| Ok((row.get::<_, u32>(0)? as usize, row.get::<_, u32>(1)? as usize)),
        ).map_err(index_error)
    }

    fn scan_workspace(&mut self) -> Result<usize, StorageError> {
        // Saves publish directly to the index. Keep changes that finish during this scan.
        let watermark: i64 = self
            .connection
            .query_row("SELECT coalesce(max(rowid), 0) FROM notes", [], |row| {
                row.get(0)
            })
            .map_err(index_error)?;
        self.connection
            .execute_batch(
                "CREATE TEMP TABLE IF NOT EXISTS dirty_before_scan (
                    relative_path TEXT PRIMARY KEY, revision TEXT NOT NULL, captured_at TEXT
                 );
                 DELETE FROM temp.dirty_before_scan;
                 INSERT INTO temp.dirty_before_scan
                 SELECT relative_path, revision, captured_at FROM search_documents WHERE mtime_ns = 0;",
            )
            .map_err(index_error)?;
        let mut captured = HashMap::new();
        {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT relative_path, captured_at, id, source_id, block_id
                 FROM notes WHERE rowid <= ?1 ORDER BY captured_at, rowid",
                )
                .map_err(index_error)?;
            let mut rows = statement.query([watermark]).map_err(index_error)?;
            while let Some(row) = rows.next().map_err(index_error)? {
                let path: String = row.get(0).map_err(index_error)?;
                let date: String = row.get(1).map_err(index_error)?;
                let record_id: String = row.get(2).map_err(index_error)?;
                let source_id: String = row.get(3).map_err(index_error)?;
                let block_id: Option<String> = row.get(4).map_err(index_error)?;
                let header = block_id
                    .is_none()
                    .then(|| capture_header(&record_id, &source_id));
                captured.insert(path, (date, header));
            }
        }
        let mut existing = HashMap::new();
        {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT relative_path, captured_at, mtime_ns, file_size FROM search_documents",
                )
                .map_err(index_error)?;
            let mut rows = statement.query([]).map_err(index_error)?;
            while let Some(row) = rows.next().map_err(index_error)? {
                let path: String = row.get(0).map_err(index_error)?;
                existing.insert(
                    path,
                    CachedDocMeta {
                        captured_at: row.get(1).map_err(index_error)?,
                        mtime_ns: row.get(2).map_err(index_error)?,
                        file_size: row.get(3).map_err(index_error)?,
                    },
                );
            }
        }
        let (entries, mut skipped) = self.files.markdown_entries().map_err(source_error)?;
        let racy_cutoff = racy_cutoff_filetime();
        let mut read_count = 0usize;
        let staging = self.connection.transaction().map_err(index_error)?;
        staging
            .execute_batch(
                "CREATE TEMP TABLE IF NOT EXISTS scanned_paths (relative_path TEXT PRIMARY KEY);
                 DELETE FROM temp.scanned_paths;
                 CREATE TEMP TABLE IF NOT EXISTS scanned_changes (
                     relative_path TEXT PRIMARY KEY,
                     captured_at TEXT,
                     revision TEXT NOT NULL,
                     body TEXT NOT NULL,
                     mtime_ns INTEGER NOT NULL,
                     file_size INTEGER NOT NULL
                 );
                 DELETE FROM temp.scanned_changes;",
            )
            .map_err(index_error)?;
        {
            let mut insert_path = staging
                .prepare("INSERT INTO temp.scanned_paths VALUES (?1)")
                .map_err(index_error)?;
            let mut insert_change = staging
                .prepare("INSERT INTO temp.scanned_changes VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
                .map_err(index_error)?;
            for entry in entries {
                let capture = captured.remove(&entry.relative_path);
                let captured_at = capture.as_ref().map(|(date, _)| date.as_str());
                let unchanged = existing.get(&entry.relative_path).is_some_and(|cached| {
                    cached.mtime_ns > 0
                        && cached.mtime_ns == entry.mtime_ns
                        && cached.file_size == entry.file_size
                        && entry.mtime_ns <= racy_cutoff
                        && cached.captured_at.as_deref() == captured_at
                });
                if unchanged {
                    insert_path
                        .execute([&entry.relative_path])
                        .map_err(index_error)?;
                    continue;
                }
                read_count += 1;
                let bytes = match self.files.read_markdown(&entry.relative_path) {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                let Ok(text) = std::str::from_utf8(&bytes) else {
                    skipped += 1;
                    continue;
                };
                let body = capture
                    .as_ref()
                    .and_then(|(_, header)| header.as_ref())
                    .and_then(|header| text.strip_prefix(header))
                    .unwrap_or(text);
                insert_path
                    .execute([&entry.relative_path])
                    .map_err(index_error)?;
                insert_change
                    .execute(params![
                        entry.relative_path,
                        captured_at,
                        revision_hash(&bytes),
                        body,
                        entry.mtime_ns,
                        entry.file_size,
                    ])
                    .map_err(index_error)?;
            }
        }
        staging.commit().map_err(index_error)?;
        self.publish_scan(watermark, skipped + captured.len())?;
        Ok(read_count)
    }

    fn publish_scan(&mut self, watermark: i64, skipped: usize) -> Result<(), StorageError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(index_error)?;
        tx.execute(
            "DELETE FROM temp.scanned_changes WHERE relative_path IN
                 (SELECT relative_path FROM notes WHERE rowid > ?1)
             OR relative_path IN (
                 SELECT d.relative_path FROM search_documents d
                 WHERE d.mtime_ns = 0 AND NOT EXISTS (
                     SELECT 1 FROM temp.dirty_before_scan old WHERE old.relative_path = d.relative_path
                       AND old.revision = d.revision AND old.captured_at IS d.captured_at
                 )
             )",
            [watermark],
        )
        .map_err(index_error)?;
        tx.execute(
            "DELETE FROM search_documents AS d
             WHERE d.relative_path NOT IN (SELECT relative_path FROM temp.scanned_paths)
               AND d.relative_path NOT IN (SELECT relative_path FROM notes WHERE rowid > ?1)
               AND (d.mtime_ns <> 0 OR EXISTS (
                   SELECT 1 FROM temp.dirty_before_scan old WHERE old.relative_path = d.relative_path
                     AND old.revision = d.revision AND old.captured_at IS d.captured_at
               ))",
            [watermark],
        ).map_err(index_error)?;
        tx.execute_batch(
            "INSERT INTO search_documents (relative_path, captured_at, revision, body, mtime_ns, file_size)
             SELECT relative_path, captured_at, revision, body, mtime_ns, file_size FROM temp.scanned_changes WHERE true
             ON CONFLICT(relative_path) DO UPDATE SET
                 captured_at = excluded.captured_at,
                 revision = excluded.revision,
                 body = excluded.body,
                 mtime_ns = excluded.mtime_ns,
                 file_size = excluded.file_size
             WHERE revision <> excluded.revision OR captured_at IS NOT excluded.captured_at;
             UPDATE search_documents
             SET mtime_ns = c.mtime_ns, file_size = c.file_size
             FROM temp.scanned_changes c
             WHERE search_documents.relative_path = c.relative_path
               AND (search_documents.mtime_ns <> c.mtime_ns OR search_documents.file_size <> c.file_size);",
        )
        .map_err(index_error)?;
        tx.execute(
            "INSERT INTO search_status VALUES (1, 1, ?1)
            ON CONFLICT(id) DO UPDATE SET generation = generation + 1, skipped = excluded.skipped",
            [skipped as i64],
        )
        .map_err(index_error)?;
        tx.commit().map_err(index_error)
    }

    pub(crate) fn search_records(
        &mut self,
        query: &str,
        from_utc: Option<&str>,
        until_utc: Option<&str>,
        offset: u32,
    ) -> Result<SearchPage, StorageError> {
        self.check_interval(from_utc, until_utc)?;
        self.ensure_index_fresh()?;
        let snapshot = self.connection.transaction().map_err(index_error)?;
        let skipped: u32 = snapshot
            .query_row(
                "SELECT skipped FROM search_status WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .map_err(index_error)?;
        let trimmed = query.trim();
        let mut hits = if let Some(fts_phrase) = fts_trigram_phrase(trimmed) {
            let mut statement = snapshot
                .prepare(
                    "SELECT d.relative_path, d.captured_at, d.revision,
                         substr(d.body, max(1, instr(lower(d.body), lower(?1)) - ?5), ?6),
                         substr(ltrim(d.body, ' ' || char(9, 10, 13, 12288, 65279)), 1, ?8)
                     FROM search_documents d
                     WHERE d.rowid IN (SELECT rowid FROM search_documents_fts WHERE search_documents_fts MATCH ?9)
                       AND (?2 IS NULL OR d.captured_at >= ?2) AND (?3 IS NULL OR d.captured_at < ?3)
                       AND (instr(lower(d.relative_path), lower(?1)) > 0 OR instr(lower(d.body), lower(?1)) > 0)
                     ORDER BY d.captured_at DESC, d.relative_path LIMIT ?7 OFFSET ?4",
                )
                .map_err(index_error)?;
            statement
                .query_map(
                    params![
                        trimmed,
                        from_utc,
                        until_utc,
                        offset,
                        LEADING_CONTEXT_CHARS,
                        EXCERPT_CHARS,
                        (PAGE_SIZE + 1) as i64,
                        TITLE_CHARS,
                        fts_phrase,
                    ],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .map_err(index_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(index_error)?
        } else {
            let mut statement = snapshot
                .prepare(
                    "SELECT relative_path, captured_at, revision,
                         substr(body, max(1, instr(lower(body), lower(?1)) - ?5), ?6),
                         substr(ltrim(body, ' ' || char(9, 10, 13, 12288, 65279)), 1, ?8)
                     FROM search_documents
                     WHERE (?2 IS NULL OR captured_at >= ?2) AND (?3 IS NULL OR captured_at < ?3)
                       AND (?1 = '' OR instr(lower(relative_path), lower(?1)) > 0 OR instr(lower(body), lower(?1)) > 0)
                     ORDER BY captured_at DESC, relative_path LIMIT ?7 OFFSET ?4",
                )
                .map_err(index_error)?;
            statement
                .query_map(
                    params![
                        trimmed,
                        from_utc,
                        until_utc,
                        offset,
                        LEADING_CONTEXT_CHARS,
                        EXCERPT_CHARS,
                        (PAGE_SIZE + 1) as i64,
                        TITLE_CHARS,
                    ],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .map_err(index_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(index_error)?
        };
        let more = hits.len() > PAGE_SIZE;
        hits.truncate(PAGE_SIZE);
        Ok((hits, more, skipped as usize))
    }

    fn check_interval(&self, from: Option<&str>, until: Option<&str>) -> Result<(), StorageError> {
        let valid: bool = self
            .connection
            .query_row(
                "SELECT (?1 IS NULL OR strftime('%Y-%m-%dT%H:%M:%fZ', ?1, '+0 seconds') IS ?1)
                AND (?2 IS NULL OR strftime('%Y-%m-%dT%H:%M:%fZ', ?2, '+0 seconds') IS ?2)
                AND (?1 IS NULL OR ?2 IS NULL OR ?1 < ?2)",
                [from, until],
                |r| r.get(0),
            )
            .map_err(index_error)?;
        if valid {
            Ok(())
        } else {
            Err(error(
                "INVALID_INPUT",
                "search",
                None,
                false,
                "记录日期范围无效",
                "invalid UTC interval",
            ))
        }
    }

    pub(crate) fn read_record_source(
        &self,
        relative_path: &str,
        expected_hash: &str,
    ) -> Result<(String, String, String), StorageError> {
        if expected_hash.len() != 64 || !expected_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(error(
                "INVALID_INPUT",
                "read_source",
                None,
                false,
                "原文版本无效",
                "invalid SHA-256",
            ));
        }
        let file_path = physical_path(relative_path);
        let capture: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT id, source_id FROM notes WHERE relative_path = ?1 AND block_id IS NULL",
                [file_path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(index_error)?;
        if capture.is_none() && !self.is_capture_path(file_path) {
            let indexed: bool = self
                .connection
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM search_documents WHERE relative_path = ?1)",
                    [file_path],
                    |r| r.get(0),
                )
                .map_err(index_error)?;
            if !indexed {
                return Err(error(
                    "SOURCE_UNAVAILABLE",
                    "read_source",
                    None,
                    false,
                    "原文已不在当前工作区索引中",
                    "source is outside this index",
                ));
            }
        }
        let bytes = read_source_bytes(&self.files, relative_path).map_err(source_error)?;
        let revision = revision_hash(&bytes);
        let mut body = String::from_utf8(bytes).map_err(|e| {
            error(
                "SOURCE_UNAVAILABLE",
                "read_source",
                None,
                false,
                "原文不是有效的 UTF-8 Markdown",
                e.utf8_error(),
            )
        })?;
        if let Some((id, source)) = capture {
            let header = capture_header(&id, &source);
            if body.starts_with(&header) {
                body.drain(..header.len());
            }
        }
        let state = if revision.eq_ignore_ascii_case(expected_hash) {
            "current"
        } else {
            "changed"
        };
        Ok((state.to_owned(), revision, body))
    }
}

fn source_error(detail: std::io::Error) -> StorageError {
    let message = match detail.kind() {
        ErrorKind::NotFound => "找不到原文，文件可能已移动或删除",
        _ => "当前无法读取原文，请检查文件占用、权限和工作区范围",
    };
    error(
        "SOURCE_UNAVAILABLE",
        "read_source",
        None,
        true,
        message,
        detail,
    )
}

pub(super) fn upsert_document(
    tx: &Connection,
    path: &str,
    captured_at: Option<&str>,
    revision: &str,
    body: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO search_documents (relative_path, captured_at, revision, body, mtime_ns, file_size)
         VALUES (?1, ?2, ?3, ?4, 0, 0)
         ON CONFLICT(relative_path) DO UPDATE SET captured_at = excluded.captured_at,
              revision = excluded.revision, body = excluded.body, mtime_ns = 0, file_size = 0",
        params![path, captured_at, revision, body],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::TestDir;
    use std::fs;

    fn open(dir: &TestDir) -> Storage {
        Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap()
    }

    #[test]
    fn indexes_workspace_markdown_and_reads_the_current_version_within_scope() {
        let dir = TestDir::new();
        let outside = TestDir::new();
        let mut storage = open(&dir);
        let folder = dir.0.join("历史");
        fs::create_dir(&folder).unwrap();
        let path = "历史/周会.MD";
        let original = "\u{feff}---\r\nowner: 我\r\n---\r\n外部原文 %_\\ Project";
        fs::write(dir.0.join(path), original).unwrap();
        fs::write(dir.0.join(".flash-test/confidential.md"), "受保护").unwrap();
        fs::write(dir.0.join("~$temp.md"), "临时").unwrap();
        fs::write(dir.0.join("bad.md"), [0xff, 0xfe]).unwrap();
        fs::write(outside.0.join("private.md"), "区外").unwrap();
        fs::hard_link(outside.0.join("private.md"), folder.join("链接.md")).unwrap();
        storage.save_draft("draft", 1, "应用笔记").unwrap();
        let saved = storage
            .save_record("capture", "draft", 1, "应用笔记")
            .unwrap();
        let mut search = storage.search_reader().unwrap();
        let (hits, more, skipped) = search.search_records("", None, None, 0).unwrap();
        assert_eq!((hits.len(), more, skipped), (3, false, 1));
        assert_eq!(hits[0].0, saved.relative_path);
        assert!(hits[0].3.contains("应用笔记"));
        assert_eq!(
            hits[1],
            (
                path.into(),
                None,
                revision_hash(original.as_bytes()),
                original.into(),
                original.trim_start_matches('\u{feff}').into()
            )
        );
        for term in ["文", "原文", "%_\\", "project", "周会"] {
            let result = search.search_records(term, None, None, 0).unwrap();
            assert_eq!(result.0, hits[1..2]);
        }
        assert!(
            search
                .search_records("原文", Some("2000-01-01T00:00:00.000Z"), None, 0)
                .unwrap()
                .0
                .is_empty()
        );
        let date = hits[0].1.as_deref().unwrap();
        assert_eq!(
            search.search_records("", Some(date), None, 0).unwrap().0,
            hits[0..1]
        );
        assert!(
            search
                .search_records("", None, Some(date), 0)
                .unwrap()
                .0
                .is_empty()
        );
        for (from, until) in [
            (Some(date), Some(date)),
            (Some("2026-02-30T00:00:00.000Z"), None),
            (Some(""), None),
            (Some("2026-01-01"), None),
        ] {
            assert_eq!(
                search.search_records("", from, until, 0).unwrap_err().code,
                "INVALID_INPUT"
            );
        }
        let previous = &hits[1].2;
        assert_eq!(
            search.read_record_source(path, previous).unwrap(),
            ("current".into(), previous.clone(), original.into())
        );
        let changed = "---\n保留: 自定义 frontmatter\n---\n外部编辑";
        fs::write(dir.0.join(path), changed).unwrap();
        assert_eq!(
            search.read_record_source(path, previous).unwrap(),
            (
                "changed".into(),
                revision_hash(changed.as_bytes()),
                changed.into()
            )
        );
        assert_eq!(search.rebuild_search_index().unwrap(), (3, 1));
        assert!(
            search
                .search_records("Project", None, None, 0)
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(
            search.search_records("外部编辑", None, None, 0).unwrap().0[0].0,
            path
        );
        fs::remove_file(dir.0.join(path)).unwrap();
        assert_eq!(
            search.read_record_source(path, previous).unwrap_err().code,
            "SOURCE_UNAVAILABLE"
        );
        fs::hard_link(outside.0.join("private.md"), dir.0.join(path)).unwrap();
        assert_eq!(search.read_record_source(path, previous).unwrap().2, "区外");
        // Even a poisoned derived row cannot authorize traversal or state-directory reads.
        for outside_path in [
            ".flash-test/confidential.md",
            "../private.md",
            "C:/private.md",
        ] {
            search
                .connection
                .execute(
                    "INSERT INTO search_documents (relative_path, captured_at, revision, body) VALUES (?1, NULL, 'old', '')",
                    [outside_path],
                )
                .unwrap();
            assert!(search.read_record_source(outside_path, previous).is_err());
        }
        assert_eq!(search.rebuild_search_index().unwrap(), (3, 1));
        assert_eq!(
            storage.load_draft("draft").unwrap().unwrap().body,
            "应用笔记"
        );
        assert_eq!(
            storage.list_recent_captures().unwrap()[0].3,
            saved.source_revision
        );
        assert_eq!(
            fs::read(outside.0.join("private.md")).unwrap(),
            "区外".as_bytes()
        );
    }

    #[test]
    fn paginates_all_matches_and_reuses_the_index_after_reopening() {
        let dir = TestDir::new();
        let storage = open(&dir);
        for n in 0..=PAGE_SIZE {
            fs::write(
                dir.0.join(format!("笔记-{n:02}.md")),
                format!("{}共同检索词 {n}", "开头".repeat(100)),
            )
            .unwrap();
        }
        let mut search = storage.search_reader().unwrap();
        let (first, more, _) = search.search_records("共同检索词", None, None, 0).unwrap();
        assert_eq!(first.len(), PAGE_SIZE);
        assert!(more && first[0].3.contains("共同检索词"));
        let (second, more, _) = search
            .search_records("共同检索词", None, None, PAGE_SIZE as u32)
            .unwrap();
        assert_eq!(second.len(), 1);
        assert!(!more && !first.contains(&second[0]));
        drop(search);
        drop(storage);
        let other = TestDir::new();
        assert!(
            open(&other)
                .search_reader()
                .unwrap()
                .search_records("检索", None, None, 0)
                .unwrap()
                .0
                .is_empty()
        );
        let storage = open(&dir);
        let mut search = storage.search_reader().unwrap();
        assert_eq!(search.generation().unwrap(), Some(1));
        assert_eq!(
            search
                .search_records("共同检索词", None, None, PAGE_SIZE as u32)
                .unwrap()
                .0,
            second
        );
    }

    #[test]
    fn search_title_comes_from_the_body_before_the_hit_context() {
        let dir = TestDir::new();
        let storage = open(&dir);
        let body = format!("\n　\n# 回查标题\n\n{}\n唯一命中词", "内容 ".repeat(200));
        fs::write(dir.0.join("search-title.md"), body).unwrap();
        let mut search = storage.search_reader().unwrap();
        let (matches, _, _) = search.search_records("唯一命中词", None, None, 0).unwrap();
        assert_eq!(matches.len(), 1);
        assert!(matches[0].3.contains("唯一命中词"));
        assert_eq!(matches[0].4.lines().next(), Some("# 回查标题"));
    }

    #[test]
    fn publishing_an_older_scan_keeps_captures_and_wiki_updates() {
        let dir = TestDir::new();
        let mut storage = open(&dir);
        let wiki = "existing-wiki.md";
        storage.create_wiki_target(wiki).unwrap();
        let mut scanner = storage.search_reader().unwrap();
        scanner.rebuild_search_index().unwrap();
        let mut search = storage.search_reader().unwrap();
        for n in 1..=2 {
            let body = format!("并发保存 {n}");
            storage.save_draft("draft", n, &body).unwrap();
            storage
                .save_record(&n.to_string(), "draft", n, &body)
                .unwrap();
            if n == 1 {
                assert_eq!(
                    search
                        .search_records("并发", None, None, 0)
                        .unwrap()
                        .0
                        .len(),
                    1
                );
            }
        }
        let suggestion = storage
            .create_wiki_suggestion(None, wiki, "索引期间的建议", "更新索引")
            .unwrap();
        storage.apply_wiki_suggestion(&suggestion.id).unwrap();
        storage.create_wiki_target("new-wiki.md").unwrap();

        // Re-publish the staged view from before the notes and Wiki writes.
        scanner.publish_scan(0, 0).unwrap();
        let hits = search.search_records("并发保存", None, None, 0).unwrap().0;
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].1.is_some()
                && hits[0].3.contains("并发保存 1")
                && hits[0].3.contains("并发保存 2")
        );
        assert_eq!(
            search
                .search_records("索引期间的建议", None, None, 0)
                .unwrap()
                .0[0]
                .0,
            wiki
        );
        assert_eq!(
            search.search_records("new-wiki", None, None, 0).unwrap().0[0].0,
            "new-wiki.md"
        );
    }

    #[test]
    fn wal_and_busy_timeout_allow_concurrent_rebuild_and_draft_writes() {
        let dir = TestDir::new();
        for index in 0..40 {
            fs::write(
                dir.0.join(format!("doc-{index:02}.md")),
                format!("# 并发样本 {index}\n正文内容用于验证重建索引与草稿写入并发"),
            )
            .unwrap();
        }
        let mut storage = open(&dir);
        let journal_mode: String = storage
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

        let mut reader = storage.search_reader().unwrap();
        let worker = std::thread::spawn(move || {
            for _ in 0..8 {
                let (indexed, _skipped) = reader.rebuild_search_index().unwrap();
                assert!(indexed >= 40);
                let hits = reader.search_records("并发样本", None, None, 0).unwrap().0;
                assert!(!hits.is_empty());
            }
        });

        for revision in 1..=30 {
            let body = format!("并发草稿版本 {revision}");
            assert_eq!(
                storage.save_draft("draft", revision, &body).unwrap(),
                revision
            );
            if revision % 10 == 0 {
                let request_id = format!("concurrent-{revision}");
                storage
                    .save_record(&request_id, "draft", revision, &body)
                    .unwrap();
            }
        }

        worker.join().unwrap();
        let mut verify = storage.search_reader().unwrap();
        assert_eq!(verify.rebuild_search_index().unwrap(), (41, 0));
        let hits = verify
            .search_records("并发草稿版本", None, None, 0)
            .unwrap()
            .0;
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn incremental_rebuild_skips_unchanged_and_updates_modified_files() {
        use std::time::{Duration, SystemTime};

        let dir = TestDir::new();
        let old_time = SystemTime::now() - Duration::from_secs(30);
        for index in 0..20 {
            let path = dir.0.join(format!("note-{index:02}.md"));
            fs::write(
                &path,
                format!("# 历史笔记 {index}\n正文关键词 增量测试 {index}"),
            )
            .unwrap();
            let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_modified(old_time).unwrap();
        }
        let storage = open(&dir);
        let mut reader = storage.search_reader().unwrap();
        assert_eq!(reader.scan_workspace().unwrap(), 20);
        // All 20 files are older than the 2s racy-clean window and unchanged -> 0 disk reads.
        assert_eq!(reader.scan_workspace().unwrap(), 0);

        let modified = dir.0.join("note-07.md");
        fs::write(&modified, "# 历史笔记 7\n已更新的增量正文 唯一变更词").unwrap();
        assert_eq!(reader.scan_workspace().unwrap(), 1);
        let hits = reader
            .search_records("唯一变更词", None, None, 0)
            .unwrap()
            .0;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "note-07.md");
        assert!(
            reader
                .search_records("增量测试 7", None, None, 0)
                .unwrap()
                .0
                .is_empty()
        );

        {
            let file = fs::OpenOptions::new().write(true).open(&modified).unwrap();
            file.set_modified(old_time).unwrap();
        }
        // mtime changed on note-07.md -> re-read once to record new mtime, then 0 reads after.
        assert_eq!(reader.scan_workspace().unwrap(), 1);
        assert_eq!(reader.scan_workspace().unwrap(), 0);
    }

    #[test]
    fn fts5_hybrid_search_handles_cjk_short_long_and_special_punctuation() {
        let dir = TestDir::new();
        let doc = "# \"引用\" OR AND NOT *\n包含单字、双字词、三字检索词、%_%\\ 特殊符号以及 MixedCaseToken。";
        fs::write(dir.0.join("special.md"), doc).unwrap();
        fs::write(dir.0.join("other.md"), "完全无关的普通文档").unwrap();
        let storage = open(&dir);
        let mut reader = storage.search_reader().unwrap();
        for term in [
            "字",
            "双字",
            "三字检索词",
            "\"引用\"",
            "OR AND NOT *",
            "%_%\\",
            "mixedcasetoken",
            "special.md",
        ] {
            let hits = reader.search_records(term, None, None, 0).unwrap().0;
            assert_eq!(hits.len(), 1, "expected 1 hit for query `{term}`");
            assert_eq!(hits[0].0, "special.md");
        }
        assert!(
            reader
                .search_records("不存在的三字词", None, None, 0)
                .unwrap()
                .0
                .is_empty()
        );
    }

    #[test]
    #[ignore = "F7 5000-doc benchmark; run explicitly with --ignored --nocapture"]
    fn f7_five_thousand_docs_incremental_rebuild_and_fts5_benchmark() {
        use std::time::{Duration, Instant, SystemTime};

        let dir = TestDir::new();
        let old_time = SystemTime::now() - Duration::from_secs(60);
        for index in 0..5000 {
            let folder = dir.0.join(format!("bucket-{:02}", index / 250));
            if index % 250 == 0 {
                fs::create_dir_all(&folder).unwrap();
            }
            let path = folder.join(format!("doc-{index:04}.md"));
            let topic = match index % 5 {
                0 => "架构评审与状态库迁移记录",
                1 => "前端组件拆分与性能优化方案",
                2 => "模型端点协议与凭据安全检查",
                3 => "任务清单锚点与冲突恢复说明",
                _ => "分歧关系追溯与引用证据核对",
            };
            let marker = if index == 4321 {
                "唯一里程碑标记词 AlphaMilestone99"
            } else {
                "常规背景笔记段落"
            };
            let body = format!(
                "# 样本 {index:04} {topic}\n\n{marker}\n{}\n",
                "本地工作区离线文档正文，用于验证大规模索引与检索性能。".repeat(8)
            );
            fs::write(&path, body).unwrap();
            let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_modified(old_time).unwrap();
        }

        let storage = open(&dir);
        let mut reader = storage.search_reader().unwrap();

        let full_start = Instant::now();
        let full_reads = reader.scan_workspace().unwrap();
        let full_elapsed = full_start.elapsed();
        assert_eq!(full_reads, 5000);

        let edited = dir.0.join("bucket-10/doc-2505.md");
        fs::write(
            &edited,
            "# 样本 2505\n增量改写后的唯一检索短语 DeltaToken42\n",
        )
        .unwrap();
        let inc_start = Instant::now();
        let inc_reads = reader.scan_workspace().unwrap();
        let inc_elapsed = inc_start.elapsed();
        assert_eq!(inc_reads, 1);

        let queries = [
            "唯一里程碑标记词",
            "AlphaMilestone99",
            "DeltaToken42",
            "状态库迁移",
            "组件拆分与性能",
            "凭据安全检查",
            "锚点与冲突恢复",
            "引用证据核对",
            "bucket-17",
            "doc-4321",
            "架构评审",
            "离线文档正文",
            "不存在的关键词组合",
            "里程碑标记",
            "DeltaToken",
            "模型端点协议",
            "分歧关系追溯",
            "任务清单锚点",
            "架构",
            "词",
        ];
        let mut fts_times = Vec::new();
        let mut all_times = Vec::new();
        for term in queries {
            let start = Instant::now();
            let (hits, _, _) = reader.search_records(term, None, None, 0).unwrap();
            let elapsed = start.elapsed();
            all_times.push(elapsed);
            if term.chars().count() >= 3 {
                fts_times.push(elapsed);
            }
            if term == "唯一里程碑标记词" || term == "AlphaMilestone99" {
                assert_eq!(hits.len(), 1);
                assert_eq!(hits[0].0, "bucket-17/doc-4321.md");
            } else if term == "DeltaToken42" {
                assert_eq!(hits.len(), 1);
                assert_eq!(hits[0].0, "bucket-10/doc-2505.md");
            }
        }
        all_times.sort();
        fts_times.sort();
        let p95_all = all_times[(all_times.len() * 95 / 100).min(all_times.len() - 1)];
        let p95_fts = fts_times[(fts_times.len() * 95 / 100).min(fts_times.len() - 1)];
        println!(
            "F7 5000-doc benchmark: full_scan={:?} (reads={}), incremental_1_edit={:?} (reads={}), 20-query p95={:?} max={:?}, FTS5(>=3 chars) p95={:?} max={:?}",
            full_elapsed,
            full_reads,
            inc_elapsed,
            inc_reads,
            p95_all,
            all_times.last().unwrap(),
            p95_fts,
            fts_times.last().unwrap(),
        );
    }
}
