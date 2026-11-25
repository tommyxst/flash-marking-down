use std::fmt::Write;

use crate::model::MemoryConfig;

use super::*;

pub(crate) struct BackgroundJob {
    pub id: String,
    pub kind: String,
    pub note_id: String,
    pub path: String,
    pub block_id: Option<String>,
}

impl Storage {
    pub(crate) fn enqueue_capture_jobs(
        &mut self,
        note_id: &str,
        organize: bool,
        memory: Option<&MemoryConfig>,
    ) -> Result<bool, StorageError> {
        if !organize && memory.is_none() {
            return Ok(false);
        }
        let db_err = |e| database_error("background_jobs", Some(note_id), e);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        // Publish both lanes together so an already-running worker can apply job priority.
        let organized = organize && Self::queue_organize(&tx, note_id).map_err(db_err)?;
        let synced = if let Some(config) = memory {
            Self::queue_memory(&tx, config, Some(note_id), false).map_err(db_err)? > 0
        } else {
            false
        };
        tx.commit().map_err(db_err)?;
        Ok(organized || synced)
    }

    pub(crate) fn enqueue_memory_sync(
        &mut self,
        config: &MemoryConfig,
        note_id: Option<&str>,
        captures_only: bool,
    ) -> Result<usize, StorageError> {
        if !config.enabled || !(config.sync_resources || config.commit_memories) {
            return Ok(0);
        }
        let db_err = |e| database_error("memory_queue", note_id, e);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let count = Self::queue_memory(&tx, config, note_id, captures_only).map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(count)
    }

    fn queue_memory(
        tx: &rusqlite::Transaction<'_>,
        config: &MemoryConfig,
        note_id: Option<&str>,
        captures_only: bool,
    ) -> rusqlite::Result<usize> {
        let mut count = 0;
        if config.sync_resources {
            count += tx.execute(
                "INSERT INTO background_jobs (id, kind, note_id, status, updated_at)
                 SELECT lower(hex(randomblob(16))), 'ov_write_resource', n.id, 'queued',
                        strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 FROM notes n
                 WHERE n.rowid IN (
                     SELECT min(candidate.rowid) FROM notes candidate
                     WHERE candidate.ov_resource_status IN ('pending', 'failed')
                       AND (?1 IS NULL OR candidate.id = ?1)
                       AND (NOT ?2 OR candidate.draft_id <> 'manual')
                       AND NOT EXISTS (
                           SELECT 1 FROM background_jobs j JOIN notes owner ON owner.id = j.note_id
                           WHERE j.kind = 'ov_write_resource' AND owner.relative_path = candidate.relative_path
                             AND (j.status = 'queued' OR (j.status = 'running' AND j.note_id = candidate.id))
                       )
                     GROUP BY candidate.relative_path
                 )
                 ORDER BY n.rowid",
                params![note_id, captures_only],
            )?;
        }
        if config.commit_memories {
            count += tx.execute(
                "INSERT INTO background_jobs (id, kind, note_id, status, updated_at)
                 SELECT lower(hex(randomblob(16))), 'ov_commit_memory', n.id, 'queued',
                        strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 FROM notes n
                 WHERE n.ov_memory_status IN ('pending', 'failed')
                   AND (?1 IS NULL OR n.id = ?1)
                   AND (NOT ?2 OR n.draft_id <> 'manual')
                   AND NOT EXISTS (
                       SELECT 1 FROM background_jobs j
                       WHERE j.kind = 'ov_commit_memory' AND j.note_id = n.id AND j.status IN ('queued', 'running')
                   )
                 ORDER BY n.rowid",
                params![note_id, captures_only],
            )?;
        }
        Ok(count)
    }

    pub(crate) fn next_background_job(
        &mut self,
        memory: Option<&MemoryConfig>,
    ) -> Result<Option<BackgroundJob>, StorageError> {
        let db_err = |e| database_error("background_jobs", None, e);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let job = tx
            .query_row(
                "SELECT j.id, j.kind, j.note_id, n.relative_path, n.block_id
                 FROM background_jobs j JOIN notes n ON n.id = j.note_id
                 WHERE j.status = 'queued' AND (
                     j.kind = 'organize' OR (?1 AND j.kind = 'ov_write_resource')
                     OR (?2 AND j.kind = 'ov_commit_memory')
                 )
                 ORDER BY CASE j.kind WHEN 'ov_write_resource' THEN 0 WHEN 'organize' THEN 1 ELSE 2 END,
                          j.rowid ASC LIMIT 1",
                params![
                    memory.is_some_and(|c| c.enabled && c.sync_resources),
                    memory.is_some_and(|c| c.enabled && c.commit_memories),
                ],
                |r| Ok(BackgroundJob {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    note_id: r.get(2)?,
                    path: r.get(3)?,
                    block_id: r.get(4)?,
                }),
            )
            .optional()
            .map_err(db_err)?;
        if let Some(job) = &job {
            tx.execute(
                "UPDATE background_jobs
                 SET status = 'running', attempts = attempts + 1, last_error = NULL,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 WHERE id = ?1",
                [&job.id],
            )
            .map_err(db_err)?;
            if job.kind == "organize" {
                tx.execute(
                    "UPDATE notes SET organize_status = 'running', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
                    [&job.note_id],
                )
                .map_err(db_err)?;
            }
        }
        tx.commit().map_err(db_err)?;
        Ok(job)
    }

    pub(crate) fn resource_batch(&self, path: &str) -> Result<(i64, String), StorageError> {
        // Take the row boundary before reading the file. A concurrent capture can have a newer job.
        let (through_row, is_daily): (i64, bool) = self
            .connection
            .query_row(
                "SELECT max(rowid), max(block_id IS NOT NULL) FROM notes WHERE relative_path = ?1",
                [path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| database_error("memory_sync", None, e))?;
        let uri = if is_daily {
            format!("daily/{}", path.rsplit('/').next().unwrap_or(path))
        } else {
            format!("notes/{path}")
        };
        Ok((through_row, uri))
    }

    pub(crate) fn read_note_markdown(&self, locator: &str) -> Result<String, String> {
        let bytes = read_source_bytes(&self.workspace.files, locator)
            .map_err(|_| "找不到笔记原文".to_owned())?;
        String::from_utf8(bytes).map_err(|_| "笔记正文不是有效的 UTF-8 Markdown".to_owned())
    }

    pub(crate) fn memory_commit_input(
        &self,
        job: &BackgroundJob,
    ) -> Result<(String, String), String> {
        let locator = note_locator(&job.path, job.block_id.as_deref());
        let raw = self.read_note_markdown(&locator)?;
        let (source_id, organize): (String, Option<String>) = self
            .connection
            .query_row(
                "SELECT source_id, organize_json FROM notes WHERE id = ?1",
                [&job.note_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "无法读取笔记的整理结果".to_owned())?;
        let body = if job.block_id.is_none() {
            raw.strip_prefix(&capture_header(&job.note_id, &source_id))
                .unwrap_or(&raw)
        } else {
            &raw
        };
        if body.trim().is_empty() {
            return Err("笔记正文为空，暂无法提交长期记忆".into());
        }
        let title = note_title(body);
        let mut markdown = format!("# {title}\n\n{body}");
        if let Some(organize) =
            organize.and_then(|raw| serde_json::from_str::<OrganizeResultDto>(&raw).ok())
        {
            if !organize.summary.trim().is_empty() {
                markdown.push_str("\n\n## 摘要\n");
                markdown.push_str(&organize.summary);
            }
            if !organize.decisions.is_empty() {
                markdown.push_str("\n\n## 决定\n");
                for decision in organize.decisions {
                    let _ = writeln!(markdown, "- {decision}");
                }
            }
            if !organize.tasks.is_empty() {
                markdown.push_str("\n## 待办\n");
                for task in organize.tasks {
                    let _ = writeln!(
                        markdown,
                        "- [{}] {}",
                        if task.done { "x" } else { " " },
                        task.text
                    );
                }
            }
        }
        Ok((title, markdown))
    }

    pub(crate) fn complete_resource_job(
        &mut self,
        job: &BackgroundJob,
        through_row: i64,
        result: Result<(String, String), String>,
    ) -> Result<(), StorageError> {
        let db_err = |e| database_error("memory_sync", Some(&job.note_id), e);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let (job_status, note_status, uri, last_error) = match result {
            Ok((uri, sent)) => {
                // A checkbox can change the file while the HTTP request is in flight.
                let unchanged = self
                    .workspace
                    .files
                    .read_markdown(&job.path)
                    .is_ok_and(|current| current == sent.as_bytes());
                let status = if unchanged { "synced" } else { "pending" };
                ("done", status, Some(uri), None)
            }
            Err(message) => ("failed", "failed", None, Some(message)),
        };
        tx.execute(
            "UPDATE notes SET ov_resource_status = ?3, ov_resource_uri = coalesce(?4, ov_resource_uri),
                              updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE relative_path = ?1 AND rowid <= ?2
               AND (?4 IS NOT NULL OR ov_resource_status <> 'synced')",
            params![job.path, through_row, note_status, uri],
        ).map_err(db_err)?;
        tx.execute(
            "UPDATE background_jobs SET status = ?3, last_error = ?4, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE kind = 'ov_write_resource' AND status IN ('queued', 'running', 'failed')
               AND note_id IN (SELECT id FROM notes WHERE relative_path = ?1 AND rowid <= ?2)
               AND (?5 OR id = ?6)",
            params![job.path, through_row, job_status, last_error, note_status != "pending", job.id],
        ).map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    pub(crate) fn complete_memory_job(
        &mut self,
        job: &BackgroundJob,
        result: Result<Option<String>, String>,
    ) -> Result<(), StorageError> {
        let db_err = |e| database_error("memory_sync", Some(&job.note_id), e);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let (job_status, note_status, task_id, last_error) = match result {
            Ok(task_id) => ("done", "synced", task_id, None),
            Err(message) => ("failed", "failed", None, Some(message)),
        };
        tx.execute(
            "UPDATE background_jobs SET status = ?2, last_error = ?3, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
            params![job.id, job_status, last_error],
        ).map_err(db_err)?;
        tx.execute(
            "UPDATE notes SET ov_memory_status = ?2, ov_task_id = ?3, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
            params![job.note_id, note_status, task_id],
        ).map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    pub(crate) fn memory_sync_error(
        &self,
        note_id: &str,
        path: &str,
        mut resource_failed: bool,
        mut memory_failed: bool,
    ) -> Result<Option<String>, StorageError> {
        if !(resource_failed || memory_failed) {
            return Ok(None);
        }
        if let Ok(Some(config)) = self.service_config::<MemoryConfig>("memory") {
            resource_failed &= config.enabled && config.sync_resources;
            memory_failed &= config.enabled && config.commit_memories;
        }
        self.connection
            .query_row(
                "SELECT j.last_error FROM background_jobs j JOIN notes n ON n.id = j.note_id
                 WHERE j.status = 'failed' AND (
                     (?3 AND n.relative_path = ?2 AND j.kind = 'ov_write_resource')
                     OR (?4 AND n.id = ?1 AND j.kind = 'ov_commit_memory')
                 )
                 ORDER BY j.updated_at DESC, j.rowid DESC LIMIT 1",
                params![note_id, path, resource_failed, memory_failed],
                |r| r.get(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(|e| database_error("memory_sync", Some(note_id), e))
    }
}
