use std::time::Duration;

use super::*;

#[derive(Clone, Copy)]
pub(crate) enum CaptureMode {
    Only,
    Queue(u32),
    Now,
}

impl CaptureMode {
    pub(crate) fn parse(mode: &str, seconds: u32) -> Result<Self, StorageError> {
        match mode {
            "only" => Ok(Self::Only),
            "queue" => Ok(Self::Queue(seconds)),
            "now" => Ok(Self::Now),
            _ => Err(invalid(STAGE, "无效的随手记整理方式")),
        }
    }
}

fn now_ms(conn: &Connection) -> Result<i64, StorageError> {
    conn.query_row(
        "SELECT CAST(unixepoch('subsec') * 1000 AS INTEGER)",
        [],
        |row| row.get(0),
    )
    .map_err(db)
}

pub(crate) fn schedule_capture(
    conn: &Connection,
    note_id: &str,
    queued_at: &str,
    mode: CaptureMode,
) -> Result<(), StorageError> {
    let seconds = match mode {
        CaptureMode::Only => return Ok(()),
        CaptureMode::Queue(seconds) => i64::from(seconds),
        CaptureMode::Now => 0,
    };
    let now = now_ms(conn)?;
    let due_at = if matches!(mode, CaptureMode::Now) {
        flush(conn, now)?;
        now
    } else {
        // New captures can join a waiting batch. The first item sets its deadline.
        conn.query_row(
            "SELECT coalesce(min(due_at), ?2) FROM capture_buffer_queue WHERE due_at > ?1",
            params![now, now + seconds * 1000],
            |row| row.get(0),
        )
        .map_err(db)?
    };
    conn.execute(
        "INSERT INTO capture_buffer_queue(note_id, queued_at, due_at) VALUES (?1, ?2, ?3)",
        params![note_id, queued_at, due_at],
    )
    .map_err(db)?;
    Ok(())
}

fn flush(conn: &Connection, now: i64) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE capture_buffer_queue SET due_at = ?1 WHERE due_at IS NULL OR due_at > ?1",
        [now],
    )
    .map_err(db)?;
    Ok(())
}

impl Storage {
    pub(crate) fn capture_receipt(
        &self,
        record: SavedRecord,
    ) -> Result<CaptureCommitResultDto, StorageError> {
        Ok(CaptureCommitResultDto {
            path: record.relative_path,
            queued: self.buffer_has(&record.record_id)?,
            note_id: record.record_id,
            buffer_seconds_left: self.buffer_seconds_left()?,
        })
    }

    fn buffer_has(&self, note: &str) -> Result<bool, StorageError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM capture_buffer_queue WHERE note_id = ?1)",
                [note],
                |row| row.get(0),
            )
            .map_err(db)
    }

    pub(crate) fn cancel_buffer_item(&self, id: &str) -> Result<(), StorageError> {
        let now = now_ms(&self.connection)?;
        let removed = self.connection.execute(
            "DELETE FROM capture_buffer_queue WHERE note_id = ?1 AND (due_at IS NULL OR due_at > ?2)",
            params![id, now],
        ).map_err(db)?;
        if removed == 0 && self.buffer_has(id)? {
            return Err(conflict(
                STAGE,
                "这次整理已经开始，请等待完成后通过整理记录撤销",
            ));
        }
        Ok(())
    }

    pub(crate) fn flush_buffer_now(&self) -> Result<(), StorageError> {
        flush(&self.connection, now_ms(&self.connection)?)
    }

    pub(crate) fn next_buffer_delay(&self) -> Result<Option<Duration>, StorageError> {
        let due: Option<i64> = self
            .connection
            .query_row("SELECT min(due_at) FROM capture_buffer_queue", [], |row| {
                row.get(0)
            })
            .map_err(db)?;
        let now = now_ms(&self.connection)?;
        Ok(due.map(|due| Duration::from_millis(due.saturating_sub(now).max(0) as u64)))
    }

    pub(crate) fn buffer_seconds_left(&self) -> Result<u32, StorageError> {
        let remaining = self.next_buffer_delay()?.unwrap_or_default();
        Ok(remaining
            .as_millis()
            .div_ceil(1000)
            .min(u128::from(u32::MAX)) as u32)
    }

    pub(super) fn buffer_queue(&self) -> Result<Vec<BufferQueueItemDto>, StorageError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT q.note_id, n.relative_path, n.title, n.body_preview, q.queued_at
             FROM capture_buffer_queue q JOIN notes n ON n.id = q.note_id ORDER BY q.rowid",
            )
            .map_err(db)?;
        statement
            .query_map([], |row| {
                let note_id: String = row.get(0)?;
                Ok(BufferQueueItemDto {
                    id: note_id.clone(),
                    note_id,
                    path: row.get(1)?,
                    title: row.get(2)?,
                    preview: row.get(3)?,
                    queued_at: row.get(4)?,
                })
            })
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)
    }

    pub(crate) fn due_buffer_items(&self) -> Result<Vec<String>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT note_id FROM capture_buffer_queue WHERE due_at <= ?1 ORDER BY due_at, rowid",
        ).map_err(db)?;
        statement
            .query_map([now_ms(&self.connection)?], |row| row.get(0))
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)
    }

    pub(crate) fn prepare_buffer_batch(&self, ids: &[String]) -> Result<BufferBatch, StorageError> {
        let mut sources = Vec::with_capacity(ids.len());
        for id in ids {
            let note = self.get_note_detail(id)?;
            sources.push(Source {
                title: note.title,
                path: note.locator,
                text: note.body,
            });
        }
        Ok(BufferBatch {
            ids: ids.to_vec(),
            sources,
        })
    }

    pub(crate) fn pause_buffer_items(&mut self, ids: &[String]) -> Result<(), StorageError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let mut statement = tx
            .prepare("UPDATE capture_buffer_queue SET due_at = NULL WHERE note_id = ?1")
            .map_err(db)?;
        for id in ids {
            statement.execute([id]).map_err(db)?;
        }
        drop(statement);
        tx.commit().map_err(db)
    }
}

pub(crate) struct BufferBatch {
    ids: Vec<String>,
    pub(crate) sources: Vec<Source>,
}

impl BufferBatch {
    pub(super) fn verify(
        &self,
        conn: &Connection,
        files: &WorkspaceFiles,
        source_count: usize,
    ) -> Result<(), StorageError> {
        let mut statement = conn.prepare("SELECT EXISTS(SELECT 1 FROM capture_buffer_queue WHERE note_id = ?1 AND due_at IS NOT NULL)")
            .map_err(db)?;
        let changed = || conflict(STAGE, "整理中的随手记已有变化，请刷新后重新整理");
        for (id, source) in self.ids.iter().zip(&self.sources).take(source_count) {
            if !statement
                .query_row([id], |row| row.get::<_, bool>(0))
                .map_err(db)?
            {
                return Err(changed());
            }
            let body =
                read_source_bytes(files, &source.path).map_err(|err| source_error(STAGE, err))?;
            if body != source.text.as_bytes() {
                return Err(changed());
            }
        }
        Ok(())
    }

    pub(super) fn finish(
        &self,
        conn: &Connection,
        compiled: &HashSet<usize>,
    ) -> Result<(), StorageError> {
        let mut remove = conn
            .prepare("DELETE FROM capture_buffer_queue WHERE note_id = ?1")
            .map_err(db)?;
        let mut keep = conn
            .prepare("UPDATE capture_buffer_queue SET due_at = NULL WHERE note_id = ?1")
            .map_err(db)?;
        for (i, id) in self.ids.iter().enumerate() {
            if compiled.contains(&i) {
                remove.execute([id]).map_err(db)?;
            } else {
                keep.execute([id]).map_err(db)?;
            }
        }
        Ok(())
    }
}
