use super::*;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS wiki_targets (path TEXT PRIMARY KEY, kind TEXT NOT NULL CHECK(kind IN ('wiki','tasks')));
CREATE UNIQUE INDEX IF NOT EXISTS one_task_list ON wiki_targets(kind) WHERE kind='tasks';
CREATE TABLE IF NOT EXISTS wiki_changes (
 id TEXT PRIMARY KEY, operation_key TEXT NOT NULL, target TEXT NOT NULL, kind TEXT NOT NULL,
 phase TEXT NOT NULL, base_hash TEXT NOT NULL, after_hash TEXT NOT NULL, before_text TEXT NOT NULL,
 after_text TEXT NOT NULL, source_path TEXT NOT NULL, source_revision TEXT NOT NULL,
 start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, source_excerpt TEXT NOT NULL, entry_text TEXT NOT NULL,
 task_id TEXT, dependency TEXT REFERENCES wiki_changes(id), undoes TEXT REFERENCES wiki_changes(id)
);
CREATE INDEX IF NOT EXISTS wiki_change_key ON wiki_changes(operation_key);";

// id, target, kind, phase, approved base, before, after, source, source version, inclusive source lines.
pub(crate) type WikiChange = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    u32,
    u32,
    String,
);
pub(crate) type WikiResult = (String, String, String);
pub(crate) type WikiTask = (String, String, String, String);
pub(crate) type WikiTaskList = (String, String, Vec<WikiTask>, Vec<String>);

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

struct Change {
    view: WikiChange,
    task_id: Option<String>,
    dependency: Option<String>,
    undoes: Option<String>,
}

fn load_change(conn: &Connection, id: &str) -> Result<Change, StorageError> {
    conn.query_row(
        "SELECT id,target,kind,phase,base_hash,before_text,after_text,source_path,source_revision,
        start_line,end_line,source_excerpt,task_id,dependency,undoes FROM wiki_changes WHERE id=?1",
        [id],
        |r| {
            Ok(Change {
                view: (
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                ),
                task_id: r.get(12)?,
                dependency: r.get(13)?,
                undoes: r.get(14)?,
            })
        },
    )
    .optional()
    .map_err(db)?
    .ok_or_else(|| invalid("找不到当前工作区的变更"))
}

fn operation_key(fields: &[&str]) -> String {
    let mut hash = Sha256::new();
    for field in fields {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn write_preview(
    conn: &mut Connection,
    key: &str,
    change: Change,
    entry: &str,
) -> Result<WikiChange, StorageError> {
    let (_, target, kind, _, base, before, after, source, version, start, end, excerpt) =
        change.view;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db)?;
    let previous: Option<String> = tx.query_row(
        "SELECT id FROM wiki_changes WHERE operation_key=?1 AND
          (phase IN ('rejected','applied','reverted','prepared') OR (base_hash=?2 AND phase IN ('pending','deferred')))
         ORDER BY rowid DESC LIMIT 1", params![key,base], |r| r.get(0)).optional().map_err(db)?;
    if let Some(id) = previous {
        return Ok(load_change(&tx, &id)?.view);
    }
    tx.execute("UPDATE wiki_changes SET phase='superseded' WHERE operation_key=?1 AND phase IN ('pending','deferred','conflict')",[key]).map_err(db)?;
    let id: String = tx.query_row("INSERT INTO wiki_changes (id,operation_key,target,kind,phase,base_hash,after_hash,
      before_text,after_text,source_path,source_revision,start_line,end_line,source_excerpt,entry_text,task_id,dependency,undoes)
      VALUES (lower(hex(randomblob(16))),?1,?2,?3,'pending',?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) RETURNING id",
      params![key,target,kind,base,revision_hash(after.as_bytes()),before,after,source,version,start,end,excerpt,entry,change.task_id,change.dependency,change.undoes], |r|r.get(0)).map_err(db)?;
    tx.commit().map_err(db)?;
    Ok(load_change(conn, &id)?.view)
}

impl Storage {
    pub(crate) fn create_wiki_target(&mut self, path: &str) -> Result<(), StorageError> {
        let target = self.files.new_markdown(path).map_err(file_error)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute(
            "INSERT INTO wiki_targets(path,kind) VALUES (?1,'wiki') ON CONFLICT(path) DO NOTHING",
            [&target.key],
        )
        .map_err(db)?;
        let role: String = tx
            .query_row(
                "SELECT kind FROM wiki_targets WHERE path=?1",
                [&target.key],
                |r| r.get(0),
            )
            .map_err(db)?;
        if role != "wiki" {
            return Err(invalid("此路径已映射为任务清单"));
        }
        let _file = target.create().map_err(|e| {
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

    pub(crate) fn map_wiki_target(&mut self, kind: &str, path: &str) -> Result<(), StorageError> {
        if !matches!(kind, "tasks" | "wiki") {
            return Err(invalid("请选择 Wiki 或任务清单"));
        }
        let target = self.files.markdown_key(path).map_err(file_error)?;
        utf8(self.files.read_markdown(&target).map_err(file_error)?)?;
        let name = target.rsplit('/').next().expect("relative Markdown path");
        let captured: Option<String> = self
            .connection
            .query_row(
                "SELECT relative_path FROM capture_intents WHERE file_name = ?1 COLLATE NOCASE",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        if let Some(path) = captured {
            if self.files.markdown_key(&path).map_err(file_error)? == target {
                return Err(invalid("原始捕获笔记不能映射为写入目标"));
            }
        }
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

    fn check_wiki_target(&self, target: &str, kind: &str) -> Result<(), StorageError> {
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_wiki_change(
        &mut self,
        kind: &str,
        target: &str,
        source_path: &str,
        source_revision: &str,
        start_line: u32,
        end_line: u32,
        text: &str,
        depends_on: Option<&str>,
    ) -> Result<WikiChange, StorageError> {
        let role = match kind {
            "wiki" => "wiki",
            "task" => "tasks",
            _ => return Err(invalid("未知变更类型")),
        };
        self.check_wiki_target(target, role)?;
        let text = text.trim();
        if text.is_empty()
            || (kind == "task" && (text.contains(['\r', '\n']) || text.contains("fmd-task:")))
        {
            return Err(invalid("任务需要单行正文，变更内容不能为空"));
        }
        let (state, _, _) = self
            .search_reader()?
            .read_record_source(source_path, source_revision)?;
        if state != "current" {
            return Err(conflict("来源已变动，请重新打开原文并核对引用"));
        }
        // Register the exact source snapshot so later edits do not break the approved evidence.
        let evidence = self
            .load_document(source_path, source_revision)
            .map_err(|detail| {
                error(
                    "SOURCE_UNAVAILABLE",
                    "wiki",
                    None,
                    true,
                    "无法登记所选原文版本，请重新核对来源",
                    detail,
                )
            })?;
        let source = evidence.body;
        if start_line == 0 || end_line < start_line || end_line as usize > source.lines().count() {
            return Err(invalid("来源行范围无效"));
        }
        let excerpt = source
            .lines()
            .skip((start_line - 1) as usize)
            .take((end_line - start_line + 1) as usize)
            .collect::<Vec<_>>()
            .join("\n");
        if excerpt.trim().is_empty() {
            return Err(invalid("所选来源片段为空"));
        }
        if self.files.markdown_key(source_path).map_err(file_error)? == target {
            return Err(invalid("来源原文不能同时作为本次写入目标"));
        }
        if let Some(id) = depends_on {
            load_change(&self.connection, id)?;
        }
        let reviewed: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM wiki_changes WHERE kind=?1 AND target=?2 AND source_path=?3
             AND source_excerpt=?4 AND dependency IS ?5 AND entry_text=?6
             AND phase IN ('rejected','applied','reverted')
             ORDER BY rowid DESC LIMIT 1",
                params![kind, target, source_path, excerpt, depends_on, text],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?;
        if let Some(id) = reviewed {
            return Ok(load_change(&self.connection, &id)?.view);
        }
        let before = utf8(self.files.read_markdown(target).map_err(file_error)?)?;
        let key = operation_key(&[
            kind,
            target,
            source_path,
            source_revision,
            &start_line.to_string(),
            &end_line.to_string(),
            text,
            depends_on.unwrap_or(""),
        ]);
        let task_id = (kind == "task").then(|| key.clone());
        let newline = if before.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let citation = format!(
            "来源：{}（第 {}–{} 行） <!-- fmd-source:{} fmd-revision:{} -->",
            source_path, start_line, end_line, evidence.id, source_revision
        );
        let mut after = before.clone();
        if !after.is_empty() && !after.ends_with('\n') {
            after.push_str(newline);
        }
        if let Some(id) = &task_id {
            after.push_str(&format!(
                "- [ ] {text} · {citation} <!-- fmd-task:{id} -->{newline}"
            ));
            unique_task(&after, id)?;
        } else {
            let body = text.replace("\r\n", "\n").replace('\n', newline);
            after.push_str(&format!(
                "{newline}{body}{newline}{newline}{citation}{newline}"
            ));
        }
        let change = Change {
            view: (
                String::new(),
                target.into(),
                kind.into(),
                String::new(),
                revision_hash(before.as_bytes()),
                before,
                after,
                source_path.into(),
                source_revision.into(),
                start_line,
                end_line,
                excerpt,
            ),
            task_id,
            dependency: depends_on.map(str::to_owned),
            undoes: None,
        };
        write_preview(&mut self.connection, &key, change, text)
    }

    pub(crate) fn wiki_changes(&self) -> Result<Vec<WikiChange>, StorageError> {
        let mut q = self
            .connection
            .prepare("SELECT id FROM wiki_changes WHERE phase <> 'superseded' ORDER BY rowid DESC")
            .map_err(db)?;
        let ids = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        ids.into_iter()
            .map(|id| Ok(load_change(&self.connection, &id)?.view))
            .collect()
    }

    pub(crate) fn rebase_wiki_change(&mut self, id: &str) -> Result<WikiChange, StorageError> {
        let change = load_change(&self.connection, id)?;
        if !matches!(change.view.3.as_str(), "pending" | "deferred" | "conflict") {
            return Err(invalid("该变更无需重新比较"));
        }
        let entry: String = self
            .connection
            .query_row(
                "SELECT entry_text FROM wiki_changes WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(db)?;
        let view = change.view;
        let next = if let Some(original) = change.undoes {
            self.preview_wiki_undo(&original)?
        } else if view.2 == "status" {
            let hash = revision_hash(&self.files.read_markdown(&view.1).map_err(file_error)?);
            self.prepare_task_status(
                change
                    .task_id
                    .as_deref()
                    .ok_or_else(|| invalid("找不到任务锚点"))?,
                &hash,
                &entry,
            )?
        } else {
            self.prepare_wiki_change(
                &view.2,
                &view.1,
                &view.7,
                &view.8,
                view.9,
                view.10,
                &entry,
                change.dependency.as_deref(),
            )?
        };
        if next.0 != id {
            self.connection
                .execute(
                    "UPDATE wiki_changes SET phase='superseded' WHERE id=?1",
                    [id],
                )
                .map_err(db)?;
        }
        Ok(next)
    }

    pub(crate) fn review_wiki_change(
        &mut self,
        id: &str,
        decision: &str,
    ) -> Result<(), StorageError> {
        if !matches!(decision, "rejected" | "deferred") {
            return Err(invalid("未知审阅操作"));
        }
        if self
            .connection
            .execute(
                "UPDATE wiki_changes SET phase=?2 WHERE id=?1 AND phase IN ('pending','deferred')",
                params![id, decision],
            )
            .map_err(db)?
            == 1
        {
            Ok(())
        } else {
            Err(invalid("该项已经处理，请刷新建议列表"))
        }
    }

    pub(crate) fn apply_wiki_changes(
        &mut self,
        ids: &[String],
    ) -> Result<Vec<WikiResult>, StorageError> {
        let mut outcomes = Vec::new();
        for id in ids {
            match self.apply_wiki_change(id) {
                Ok(phase) => outcomes.push((id.clone(), phase, String::new())),
                Err(e) => outcomes.push((id.clone(), e.code.to_owned(), e.safe_message.to_owned())),
            }
        }
        Ok(outcomes)
    }

    pub(crate) fn recover_wiki_changes(&mut self) -> Result<Vec<WikiResult>, StorageError> {
        let ids = {
            let mut q = self
                .connection
                .prepare("SELECT id FROM wiki_changes WHERE phase='prepared' ORDER BY rowid")
                .map_err(db)?;
            q.query_map([], |r| r.get(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?
        };
        self.apply_wiki_changes(&ids)
    }

    fn apply_wiki_change(&mut self, id: &str) -> Result<String, StorageError> {
        let change = load_change(&self.connection, id)?;
        let (_, target, kind, phase, _, before, after, source_path, source_revision, _, _, _) =
            change.view;
        if phase == "applied" {
            return Ok(phase);
        }
        if !matches!(phase.as_str(), "pending" | "deferred" | "prepared") {
            return Err(invalid("该版本无法采用，请重新比较或查看历史"));
        }
        if let Some(dep) = &change.dependency {
            if load_change(&self.connection, dep)?.view.3 != "applied" {
                return Err(conflict("前置变更尚未采用，此目标未执行"));
            }
        }
        let role = if kind == "wiki" || kind == "undo_wiki" {
            "wiki"
        } else {
            "tasks"
        };
        self.check_wiki_target(&target, role)?;
        if matches!(kind.as_str(), "wiki" | "task") && phase != "prepared" {
            if self
                .search_reader()?
                .read_record_source(&source_path, &source_revision)?
                .0
                != "current"
            {
                return Err(conflict("来源版本已改变，请重新比较"));
            }
        }
        let mut locked = self.files.lock_markdown(&target).map_err(file_error)?;
        let file = &mut locked.file;
        let mut current = Vec::new();
        file.read_to_end(&mut current).map_err(file_error)?;
        if !(phase == "prepared" && current == after.as_bytes()) {
            if current != before.as_bytes() {
                self.connection
                    .execute("UPDATE wiki_changes SET phase='conflict' WHERE id=?1", [id])
                    .map_err(db)?;
                return Err(conflict("目标文件已有新版本，请重新比较"));
            }
            self.connection
                .execute("UPDATE wiki_changes SET phase='prepared' WHERE id=?1", [id])
                .map_err(db)?;
            #[cfg(test)]
            tests::at_write_boundary();
            let mut write_locked = |bytes: &[u8]| {
                file.seek(SeekFrom::Start(0))?;
                file.write_all(bytes)?;
                file.set_len(bytes.len() as u64)?;
                file.sync_all()
            };
            if let Err(detail) = write_locked(after.as_bytes()) {
                if let Err(rollback) = write_locked(before.as_bytes()) {
                    return Err(error(
                        "WRITE_FAILED",
                        "wiki",
                        Some(id),
                        true,
                        "写入失败且无法恢复原文件，已保留前后版本，请手工核对",
                        format!("{detail}; {rollback}"),
                    ));
                }
                return Err(file_error(detail));
            }
            current.clear();
            file.seek(SeekFrom::Start(0))
                .and_then(|_| file.read_to_end(&mut current))
                .map_err(file_error)?;
            if current != after.as_bytes() {
                return Err(conflict("写入后核对失败，已保留变更前后版本"));
            }
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute("UPDATE wiki_changes SET phase='applied' WHERE id=?1", [id])
            .map_err(db)?;
        if let Some(original) = change.undoes {
            tx.execute(
                "UPDATE wiki_changes SET phase='reverted' WHERE id=?1",
                [original],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        Ok("applied".into())
    }

    pub(crate) fn wiki_tasks(&self) -> Result<WikiTaskList, StorageError> {
        let target: String = self
            .connection
            .query_row(
                "SELECT path FROM wiki_targets WHERE kind='tasks'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?
            .ok_or_else(|| invalid("请先映射 Markdown 任务清单"))?;
        let body = utf8(self.files.read_markdown(&target).map_err(file_error)?)?;
        let (mut parsed, manual, mut notices) = parse_tasks(&body);
        let mut tasks = Vec::new();
        let mut q = self
            .connection
            .prepare(
                "SELECT task_id,entry_text,source_path FROM wiki_changes
            WHERE target=?1 AND kind='task' AND phase='applied' ORDER BY rowid",
            )
            .map_err(db)?;
        let mut rows = q.query([&target]).map_err(db)?;
        while let Some(row) = rows.next().map_err(db)? {
            let id: String = row.get(0).map_err(db)?;
            let title: String = row.get(1).map_err(db)?;
            let source: String = row.get(2).map_err(db)?;
            let (text, state) = match parsed.remove(&id) {
                Some(mut entries) if entries.len() == 1 => {
                    let entry = entries.pop().unwrap();
                    (entry.text, entry.state.to_owned())
                }
                _ => {
                    notices.push(format!("任务 {title} 的锚点缺失或重复，请核对清单"));
                    (title, "invalid".into())
                }
            };
            tasks.push((id, text, state, source));
        }
        for entries in parsed.into_values() {
            for entry in entries {
                tasks.push((String::new(), entry.text, "invalid".into(), String::new()));
                notices.push("清单含未登记来源的任务，请先核对其锚点".into());
            }
        }
        for (title, state) in manual {
            tasks.push((String::new(), title, state.to_owned(), String::new()));
        }
        Ok((target, revision_hash(body.as_bytes()), tasks, notices))
    }

    pub(crate) fn prepare_task_status(
        &mut self,
        task_id: &str,
        base_hash: &str,
        state: &str,
    ) -> Result<WikiChange, StorageError> {
        let original: String = self
            .connection
            .query_row(
                "SELECT c.id FROM wiki_changes c JOIN wiki_targets t ON t.path=c.target
            WHERE c.kind='task' AND c.phase='applied' AND c.task_id=?1 AND t.kind='tasks'",
                [task_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db)?
            .ok_or_else(|| invalid("任务没有有效的清单锚点"))?;
        let original = load_change(&self.connection, &original)?.view;
        let before = utf8(self.files.read_markdown(&original.1).map_err(file_error)?)?;
        if revision_hash(before.as_bytes()) != base_hash {
            return Err(conflict("任务清单已被修改，请刷新状态"));
        }
        let after = set_task_state(&before, task_id, state)?;
        let generation:i64=self.connection.query_row("SELECT max(rowid) FROM wiki_changes WHERE target=?1 AND phase IN ('applied','reverted')",
            [&original.1],|r|r.get(0)).map_err(db)?;
        let key = operation_key(&[
            "status",
            &original.1,
            task_id,
            base_hash,
            state,
            &generation.to_string(),
        ]);
        let change = Change {
            view: (
                String::new(),
                original.1,
                "status".into(),
                String::new(),
                base_hash.into(),
                before,
                after,
                original.7,
                original.8,
                original.9,
                original.10,
                original.11,
            ),
            task_id: Some(task_id.into()),
            dependency: None,
            undoes: None,
        };
        write_preview(&mut self.connection, &key, change, state)
    }

    pub(crate) fn preview_wiki_undo(&mut self, id: &str) -> Result<WikiChange, StorageError> {
        let original = load_change(&self.connection, id)?;
        let (_, target, kind, phase, _, before, after, source, version, start, end, excerpt) =
            original.view;
        if phase != "applied" || original.undoes.is_some() {
            return Err(invalid("只能撤销已采用的原始变更"));
        }
        self.check_wiki_target(&target, if kind == "wiki" { "wiki" } else { "tasks" })?;
        let current = utf8(self.files.read_markdown(&target).map_err(file_error)?)?;
        let inverse = if current == after {
            before
        } else if kind == "status" {
            let task_id = original
                .task_id
                .as_deref()
                .ok_or_else(|| invalid("任务锚点缺失"))?;
            let from = unique_task(&after, task_id)?.state;
            if unique_task(&current, task_id)?.state != from {
                return Err(conflict("任务状态随后已改变，请手工核对逆向差异"));
            }
            set_task_state(&current, task_id, unique_task(&before, task_id)?.state)?
        } else {
            let added = after
                .strip_prefix(&before)
                .ok_or_else(|| conflict("无法定位可独立撤销的内容"))?;
            let mut matches = current.match_indices(added);
            let Some((pos, _)) = matches.next() else {
                return Err(conflict(
                    "原变更已被编辑，无法唯一定位；请对照历史差异手工核对",
                ));
            };
            if added.is_empty() || matches.next().is_some() {
                return Err(conflict("有多个相同片段，无法安全撤销"));
            }
            let mut inverse = current.clone();
            let remaining = &current[pos + added.len()..];
            let keep_line_break = pos > 0
                && !current[..pos].ends_with('\n')
                && !remaining.is_empty()
                && !remaining.starts_with(['\r', '\n'])
                && added.ends_with('\n');
            let separator = if keep_line_break && added.ends_with("\r\n") {
                "\r\n"
            } else if keep_line_break {
                "\n"
            } else {
                ""
            };
            inverse.replace_range(pos..pos + added.len(), separator);
            inverse
        };
        let hash = revision_hash(current.as_bytes());
        let key = operation_key(&["undo", id, &hash]);
        let change = Change {
            view: (
                String::new(),
                target,
                format!("undo_{kind}"),
                String::new(),
                hash,
                current,
                inverse,
                source,
                version,
                start,
                end,
                excerpt,
            ),
            task_id: original.task_id,
            dependency: None,
            undoes: Some(id.into()),
        };
        write_preview(&mut self.connection, &key, change, "")
    }
}

struct ParsedTask {
    text: String,
    state: &'static str,
    checkbox: usize,
}

type ManualTask = (String, &'static str);

fn parse_manual_task(content: &str) -> Option<ManualTask> {
    let trimmed = content.trim_start();
    let (state, rest) = match trimmed.as_bytes().get(..6) {
        Some(b"- [ ] " | b"* [ ] ") => ("open", &trimmed[6..]),
        Some(b"- [x] " | b"- [X] " | b"* [x] " | b"* [X] ") => ("done", &trimmed[6..]),
        Some(b"- [-] " | b"* [-] ") => ("cancelled", &trimmed[6..]),
        _ => return None,
    };
    let title = rest.trim();
    (!title.is_empty()).then(|| (title.to_owned(), state))
}

fn parse_tasks(
    body: &str,
) -> (
    HashMap<String, Vec<ParsedTask>>,
    Vec<ManualTask>,
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
        let line = raw.trim_end_matches(['\r', '\n']);
        let line = if number == 0 {
            line.trim_start_matches('\u{feff}')
        } else {
            line
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
                        });
                    }
                }
            } else if let Some(task) = parse_manual_task(content) {
                manual.push(task);
            }
            continue;
        };
        let state = match head.as_bytes().get(..6) {
            Some(b"- [ ] ") => "open",
            Some(b"- [x] " | b"- [X] ") => "done",
            Some(b"- [-] ") => "cancelled",
            _ => "invalid",
        };
        let text = if state == "invalid" {
            issues.push(format!("第 {} 行的任务复选框损坏", number + 1));
            head
        } else {
            head[6..]
                .rsplit_once(" · 来源：")
                .map_or(&head[6..], |(title, _)| title)
        };
        let checkbox = line_start + raw.len() - raw.trim_start_matches('\u{feff}').len()
            + line.len()
            - content.len()
            + 3;
        found.entry(id.into()).or_default().push(ParsedTask {
            text: text.into(),
            state,
            checkbox,
        });
    }
    (found, manual, issues)
}

fn unique_task(body: &str, id: &str) -> Result<ParsedTask, StorageError> {
    let mut entries = parse_tasks(body)
        .0
        .remove(id)
        .ok_or_else(|| conflict("任务锚点已丢失，请先修复清单"))?;
    if entries.len() != 1 || entries[0].state == "invalid" {
        return Err(conflict("任务锚点重复或语法损坏，请先修复清单"));
    }
    Ok(entries.pop().unwrap())
}

fn set_task_state(body: &str, id: &str, state: &str) -> Result<String, StorageError> {
    let task = unique_task(body, id)?;
    let mark = match state {
        "open" => " ",
        "done" => "x",
        "cancelled" => "-",
        _ => return Err(invalid("未知任务状态")),
    };
    if task.state == state {
        return Err(invalid("任务已经处于该状态"));
    }
    let mut next = body.to_owned();
    next.replace_range(task.checkbox..task.checkbox + 1, mark);
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::super::tests::TestDir;
    use super::*;
    use std::{
        cell::RefCell,
        fs,
        process::Command,
        time::{Duration, Instant},
    };

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

    fn example() -> (TestDir, Storage, SavedRecord) {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        storage
            .save_draft("draft", 1, "会议讨论\n- [ ] 请复核报价")
            .unwrap();
        let saved = storage
            .save_record("example", "draft", 1, "会议讨论\n- [ ] 请复核报价")
            .unwrap();
        (dir, storage, saved)
    }
    fn preview(
        storage: &mut Storage,
        source: &SavedRecord,
        kind: &str,
        target: &str,
        entry: &str,
        depends: Option<&str>,
    ) -> WikiChange {
        storage
            .prepare_wiki_change(
                kind,
                target,
                &source.relative_path,
                &source.source_revision,
                2,
                2,
                entry,
                depends,
            )
            .unwrap()
    }

    #[test]
    fn versions_rejection_partial_application_recovery_and_inverse_preserve_files() {
        let (dir, mut storage, source) = example();
        for path in ["a.md", "b.md", "c.md"] {
            storage.create_wiki_target(path).unwrap();
        }
        fs::write(dir.0.join("a.md"), "\u{feff}人工原稿").unwrap();
        assert!(storage.create_wiki_target("a.md").is_err());
        assert!(storage.create_wiki_target(".flash-test/hidden.md").is_err());
        assert!(storage.create_wiki_target("../outside.md").is_err());
        assert!(
            storage
                .map_wiki_target("wiki", &source.relative_path)
                .is_err()
        );
        let a = preview(&mut storage, &source, "wiki", "a.md", "采用的意见", None);
        let b = preview(&mut storage, &source, "wiki", "b.md", "尚待讨论", None);
        let c = preview(
            &mut storage,
            &source,
            "wiki",
            "c.md",
            "依赖第二篇",
            Some(&b.0),
        );
        fs::write(dir.0.join("b.md"), "其他编辑器的意见").unwrap();
        let result = storage
            .apply_wiki_changes(&[a.0.clone(), b.0.clone(), c.0.clone()])
            .unwrap();
        assert_eq!(
            result.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
            ["applied", "VERSION_CONFLICT", "VERSION_CONFLICT"]
        );
        assert_eq!(
            fs::read_to_string(dir.0.join("b.md")).unwrap(),
            "其他编辑器的意见"
        );
        assert_eq!(fs::read_to_string(dir.0.join("c.md")).unwrap(), "");
        assert_eq!(storage.apply_wiki_change(&a.0).unwrap(), "applied");
        assert_eq!(fs::read_to_string(dir.0.join("a.md")).unwrap(), a.6);
        let revised = storage.rebase_wiki_change(&b.0).unwrap();
        assert_ne!(b.0, revised.0);
        assert_eq!(b.11, "- [ ] 请复核报价");
        storage.review_wiki_change(&revised.0, "deferred").unwrap();
        storage.review_wiki_change(&revised.0, "rejected").unwrap();
        assert_eq!(
            preview(&mut storage, &source, "wiki", "b.md", "尚待讨论", None).3,
            "rejected"
        );
        fs::write(dir.0.join("a.md"), format!("{}另一项手工补充\n", a.6)).unwrap();
        let undo = storage.preview_wiki_undo(&a.0).unwrap();
        assert_eq!(undo.6, "\u{feff}人工原稿\n另一项手工补充\n");
        storage
            .connection
            .execute(
                "UPDATE wiki_changes SET phase='prepared' WHERE id=?1",
                [&undo.0],
            )
            .unwrap();
        // Simulate termination after writing the after-image but before acknowledging the transaction.
        fs::write(dir.0.join("a.md"), &undo.6).unwrap();
        drop(storage);
        let mut reopened = Storage::open(&dir.0, Path::new("."), ".flash-test").unwrap();
        assert_eq!(reopened.recover_wiki_changes().unwrap()[0].1, "applied");
        assert_eq!(
            load_change(&reopened.connection, &a.0).unwrap().view.3,
            "reverted"
        );
        assert_eq!(fs::read_to_string(dir.0.join("a.md")).unwrap(), undo.6);
    }

    #[test]
    fn task_status_is_read_from_markdown_and_broken_anchors_never_write() {
        let (dir, mut storage, source) = example();
        let path = dir.0.join("Tasks.md");
        fs::write(&path, "\u{feff}---\r\ntitle: 保留\r\n---\r\n").unwrap();
        storage.map_wiki_target("tasks", "Tasks.md").unwrap();
        let item = preview(
            &mut storage,
            &source,
            "task",
            "Tasks.md",
            "请复核报价",
            None,
        );
        storage.apply_wiki_change(&item.0).unwrap();
        let (_, base, tasks, issues) = storage.wiki_tasks().unwrap();
        assert!(issues.is_empty());
        let (id, title, state, origin) = &tasks[0];
        assert_eq!(
            (title.as_str(), state.as_str(), origin.as_str()),
            ("请复核报价", "open", source.relative_path.as_str())
        );
        let edit = storage.prepare_task_status(id, &base, "done").unwrap();
        storage.apply_wiki_change(&edit.0).unwrap();
        let done = fs::read_to_string(&path).unwrap();
        assert_eq!(done, item.6.replacen("- [ ]", "- [x]", 1));
        assert!(storage.prepare_task_status(id, &base, "open").is_err());
        fs::write(&path, done.replace("- [x]", "- [-]")).unwrap();
        assert_eq!(storage.wiki_tasks().unwrap().2[0].2, "cancelled");
        let undo = storage.preview_wiki_undo(&edit.0);
        assert!(undo.is_err());
        let checking = done.lines().find(|line| line.starts_with("- [x]")).unwrap();
        for broken in [
            format!("{}{}", done, done),
            done.replace("fmd-task:", "broken:"),
            done.replace("- [x]", "- [xx]"),
            format!("{done}- 父任务\n    {checking}\n"),
            done.replace(checking, &format!("注释 <!--\n{checking}\n-->")),
        ] {
            fs::write(&path, &broken).unwrap();
            let (_, hash, tasks, notices) = storage.wiki_tasks().unwrap();
            assert_eq!(tasks[0].2, "invalid");
            assert!(!notices.is_empty());
            assert!(storage.prepare_task_status(id, &hash, "open").is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), broken);
        }
        fs::write(&path, &done).unwrap();
        let undo = storage.preview_wiki_undo(&edit.0).unwrap();
        storage.apply_wiki_change(&undo.0).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), item.6);
        fs::write(&path, format!("{done}尚未结束的注释 <!--")).unwrap();
        assert!(
            storage
                .prepare_wiki_change(
                    "task",
                    "Tasks.md",
                    &source.relative_path,
                    &source.source_revision,
                    1,
                    1,
                    "新的草拟任务",
                    None
                )
                .is_err()
        );
    }

    #[test]
    fn manual_tasks_and_links_coexist_with_anchored_tasks_without_errors() {
        let (dir, mut storage, source) = example();
        let path = dir.0.join("Tasks.md");
        fs::write(
            &path,
            "# 待办\n- [ ] 手工待办\n- [x] 已完成手工任务\n- [参考文档](ref.md)\n",
        )
        .unwrap();
        storage.map_wiki_target("tasks", "Tasks.md").unwrap();
        let item = preview(
            &mut storage,
            &source,
            "task",
            "Tasks.md",
            "应用内采用任务",
            None,
        );
        storage.apply_wiki_change(&item.0).unwrap();
        let (_, _, tasks, notices) = storage.wiki_tasks().unwrap();
        assert!(notices.is_empty());
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].1, "应用内采用任务");
        assert!(!tasks[0].0.is_empty());
        assert_eq!(
            (
                tasks[1].0.as_str(),
                tasks[1].1.as_str(),
                tasks[1].2.as_str()
            ),
            ("", "手工待办", "open")
        );
        assert_eq!(
            (
                tasks[2].0.as_str(),
                tasks[2].1.as_str(),
                tasks[2].2.as_str()
            ),
            ("", "已完成手工任务", "done")
        );
    }

    fn wait_file(path: &Path) {
        let start = Instant::now();
        while !path.exists() && start.elapsed() < Duration::from_secs(12) {
            std::thread::sleep(Duration::from_millis(15));
        }
        assert!(path.exists(), "editor did not signal {}", path.display());
    }

    #[test]
    #[ignore = "requires a real Vim executable in FMD_TEST_VIM"]
    fn real_vim_write_at_the_checked_file_boundary_preserves_human_edits() {
        let vim = std::env::var_os("FMD_TEST_VIM").expect("FMD_TEST_VIM is required");
        let (dir, mut storage, source) = example();
        storage.create_wiki_target("editor.md").unwrap();
        for round in 0..3 {
            let entry = format!("应用改动 {round}");
            let change = preview(&mut storage, &source, "wiki", "editor.md", &entry, None);
            let backup = if round % 2 == 0 {
                "set writebackup backupcopy=no"
            } else {
                "set nowritebackup nobackup"
            };
            let script = format!(
                "set nomodeline noswapfile\n{backup}\nedit editor.md\ncall append('$','human-{round}')\ncall writefile(['ready'],'ready{round}')\nlet n=0\nwhile !filereadable('go{round}') && n<600\n sleep 20m\n let n+=1\nendwhile\nlet v:errmsg=''\nsilent! write\ncall writefile([v:errmsg,string(&modified)],'attempt{round}')\nlet n=0\nwhile !filereadable('retry{round}') && n<600\n sleep 20m\n let n+=1\nendwhile\nlet human_edit=getline('$')\nedit!\ncall append('$',human_edit)\nwrite\ncall writefile([string(&modified)],'result{round}')\nqa!\n"
            );
            fs::write(dir.0.join("race.vim"), script).unwrap();
            let mut child = Command::new(&vim)
                .current_dir(&dir.0)
                .args([
                    "-Nu",
                    "NONE",
                    "-i",
                    "NONE",
                    "-n",
                    "-es",
                    "--noplugin",
                    "-S",
                    "race.vim",
                ])
                .spawn()
                .unwrap();
            wait_file(&dir.0.join(format!("ready{round}")));
            let root = dir.0.clone();
            BEFORE_WRITE.with_borrow_mut(|hook| {
                *hook = Some(Box::new(move || {
                    fs::write(root.join(format!("go{round}")), b"1").unwrap();
                    wait_file(&root.join(format!("attempt{round}")));
                }))
            });
            let phase = storage.apply_wiki_change(&change.0);
            fs::write(dir.0.join(format!("retry{round}")), b"1").unwrap();
            assert!(child.wait().unwrap().success());
            assert_eq!(phase.unwrap(), "applied");
            let attempt = fs::read_to_string(dir.0.join(format!("attempt{round}"))).unwrap();
            assert!(attempt.lines().next().is_some_and(|s| !s.is_empty()));
            assert_eq!(
                attempt.lines().nth(1),
                Some("1"),
                "Vim must retain the unsaved buffer"
            );
            assert_eq!(
                fs::read_to_string(dir.0.join(format!("result{round}")))
                    .unwrap()
                    .trim(),
                "0"
            );
            let final_text = fs::read_to_string(dir.0.join("editor.md")).unwrap();
            for i in 0..=round {
                assert!(final_text.contains(&format!("human-{i}")));
                assert!(final_text.contains(&format!("应用改动 {i}")));
            }
        }
    }
}
