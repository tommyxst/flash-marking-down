use serde_json::{Value, json};

use super::*;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS m2_analyses (id TEXT PRIMARY KEY REFERENCES jobs(id), error TEXT, items TEXT NOT NULL DEFAULT '[]');
CREATE TABLE IF NOT EXISTS m2_answers (id TEXT PRIMARY KEY, data TEXT NOT NULL);";

pub(crate) type ModelConfig = (String, String, bool);
pub(crate) type M2Result<T> = Result<T, String>;
const ANALYSIS_RULE: &str = "m2-analysis-v1";
pub(crate) const SOURCE_LIMIT: usize = 48 * 1024;
const EXCERPT_LIMIT: usize = 4800;
const ITEM_LIMIT: usize = 30;
pub(crate) struct Document {
    pub(crate) id: String,
    identity: String,
    pub(crate) path: String,
    pub(crate) revision: String,
    pub(crate) date: Option<String>,
    pub(crate) event_date: Option<String>,
    pub(crate) is_capture: bool,
    pub(crate) body: String,
}

pub(crate) enum AnalysisRequest {
    Cached(Value),
    Generate { id: String, document: Document },
}

pub(crate) fn db(error: rusqlite::Error) -> String {
    database_error("m2", None, error).safe_message.to_owned()
}

pub(crate) fn parse(data: &str) -> M2Result<Value> {
    serde_json::from_str(data).map_err(|_| "工作区整理数据无法读取".into())
}

pub(crate) fn required<'a>(value: &'a Value, key: &str) -> M2Result<&'a str> {
    value[key]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("缺少有效字段 {key}"))
}

impl Document {
    fn evidence(&self, first: u32, last: u32, outside: bool) -> M2Result<Value> {
        if first == 0 || last < first {
            return Err("来源行范围无效".into());
        }
        let lines: Vec<&str> = self
            .body
            .lines()
            .skip(first as usize - 1)
            .take((last - first) as usize + 1)
            .collect();
        if lines.len() != (last - first) as usize + 1 {
            return Err("来源行范围超出原文".into());
        }
        let excerpt = lines.join("\n");
        if excerpt.trim().is_empty() || excerpt.len() > EXCERPT_LIMIT {
            return Err("证据片段为空或过长，请选择更短的原文片段".into());
        }
        let id = revision_hash(format!("{}:{}:{first}:{last}", self.id, self.revision).as_bytes());
        Ok(
            json!({"id":id,"sourceId":self.id,"fileIdentity":self.identity,"path":self.path,"revision":self.revision,
            "startLine":first,"endLine":last,"excerpt":excerpt,"date":self.date,
            "eventDate":self.event_date,"outsideRange":outside,"available":true,"changed":false,"isCapture":self.is_capture}),
        )
    }

    pub(crate) fn locate(&self, excerpt: &str) -> M2Result<Value> {
        let needle = excerpt.trim().replace("\r\n", "\n");
        if needle.is_empty() {
            return Err("请填写可在原文中定位的片段".into());
        }
        let normalized = self.body.replace("\r\n", "\n");
        let mut matches = normalized.match_indices(&needle);
        let (start, _) = matches.next().ok_or("在原文中找不到该片段")?;
        if matches.next().is_some() {
            return Err("原文中有多处相同片段，请补充上下文后重试".into());
        }
        let first = normalized[..start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count() as u32
            + 1;
        let last = first + needle.bytes().filter(|byte| *byte == b'\n').count() as u32;
        self.evidence(first, last, false)
    }

    pub(crate) fn search_evidence(&self, keyword: &str) -> M2Result<Value> {
        let lower = keyword.to_ascii_lowercase();
        let lines: Vec<&str> = self.body.lines().collect();
        let index = lines
            .iter()
            .position(|line| line.to_ascii_lowercase().contains(&lower))
            .or_else(|| lines.iter().position(|line| !line.trim().is_empty()))
            .ok_or("原文没有可引用正文")?;
        let start = index.saturating_sub(1);
        let end = (index + 2).min(lines.len() - 1);
        self.evidence(start as u32 + 1, end as u32 + 1, false)
            .or_else(|_| self.evidence(index as u32 + 1, index as u32 + 1, false))
    }
}

impl Storage {
    pub(crate) fn service_config<T: serde::de::DeserializeOwned>(
        &self,
        slot: &str,
    ) -> M2Result<Option<T>> {
        let serialized: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM workspace_settings WHERE key=?1",
                [format!("{slot}_config")],
                |row| row.get(0),
            )
            .optional()
            .map_err(db)?;
        serialized
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| "无法读取服务配置".into())
    }

    pub(crate) fn model_config(&self) -> M2Result<Option<ModelConfig>> {
        Ok(self
            .service_config::<crate::model::ModelSettings>("model")?
            .map(|c| (c.endpoint, c.model, c.enabled)))
    }

    pub(crate) fn save_service_config(
        &self,
        slot: &str,
        config: &impl serde::Serialize,
        secret: &str,
    ) -> Result<(), crate::dto::AppError> {
        use crate::dto::AppError;
        let key = crate::credential_commands::config_key(self, slot, secret)?;
        let value = serde_json::to_string(config)
            .map_err(|_| AppError::invalid("settings", "无法编码服务配置"))?;
        let tx = self
            .connection
            .unchecked_transaction()
            .map_err(|e| database_error("settings", None, e))?;
        tx.execute(
            "INSERT OR REPLACE INTO workspace_settings(key,value) VALUES (?1,?2)",
            params![format!("{slot}_config"), value],
        )
        .map_err(|e| database_error("settings", None, e))?;
        if let Some((target, token)) = key {
            self.set_credential_reference(slot, Some(&target))?;
            crate::platform::write_credential(&target, token).map_err(|e| {
                AppError::new(
                    "AUTH_FAILED",
                    "credential",
                    true,
                    "无法写入 Windows 凭据管理器",
                    Some(e.to_string()),
                )
            })?;
        }
        tx.commit()
            .map_err(|e| database_error("settings", None, e))?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_model_config(
        &self,
        endpoint: String,
        model: String,
        enabled: bool,
    ) -> M2Result<ModelConfig> {
        let config = crate::model::ModelSettings::new(&endpoint, &model, enabled)?;
        self.save_service_config("model", &config, "")
            .map_err(|e| e.safe_message)?;
        Ok((config.endpoint, config.model, config.enabled))
    }

    pub(crate) fn approved_model(&self, approved_endpoint: &str) -> M2Result<ModelConfig> {
        let config = self
            .model_config()?
            .filter(|config| config.2)
            .ok_or("请先配置并启用模型")?;
        if config.0 != approved_endpoint {
            return Err("模型端点已改变，请重新确认资料范围".into());
        }
        Ok(config)
    }

    pub(crate) fn load_document(&mut self, path: &str, expected: &str) -> M2Result<Document> {
        let bytes = read_source_bytes(&self.workspace.files, path)
            .map_err(|_| "原文无法读取，可能已移动、删除或被占用")?;
        let revision = revision_hash(&bytes);
        if expected != revision {
            return Err("原文版本已改变，请刷新后重试".into());
        }
        let content = std::str::from_utf8(&bytes).map_err(|_| "原文不是有效的 UTF-8 Markdown")?;
        let (file_path, marker) = path
            .split_once(NOTE_ANCHOR)
            .map_or((path, None), |(file, marker)| (file, Some(marker)));
        let capture: Option<(String, String, String)> = self.connection.query_row(
            "SELECT id,source_id,captured_at FROM notes WHERE relative_path=?1 AND block_id IS ?2",
            params![file_path, marker], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional().map_err(db)?;
        let mut body = content;
        let date = if let Some((id, source, date)) = &capture {
            if marker.is_none() {
                body = content
                    .strip_prefix(&capture_header(id, source))
                    .unwrap_or(content);
            }
            Some(date.clone())
        } else {
            self.connection
                .query_row(
                    "SELECT captured_at FROM search_documents WHERE relative_path=?1",
                    [file_path],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .map_err(db)?
                .flatten()
        };
        let is_capture = capture.is_some() || date.is_some();
        let event_date = event_date(&self.connection, body)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let id: String = tx.query_row(
            "INSERT INTO sources VALUES (coalesce(?3, lower(hex(randomblob(16)))),?1,?2,'available')
             ON CONFLICT(location) DO UPDATE SET current_revision=excluded.current_revision,availability='available' RETURNING id",
            params![path,revision,capture.as_ref().map(|(_, source, _)| source)], |row| row.get(0)).map_err(db)?;
        tx.execute("INSERT OR IGNORE INTO source_revisions VALUES (?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?3,'utf-8',?4)",
            params![id,revision,bytes,newline(&bytes)]).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(Document {
            id,
            identity: path.to_owned(),
            path: path.to_owned(),
            revision,
            date,
            event_date,
            is_capture,
            body: body.to_owned(),
        })
    }

    pub(crate) fn source_is_current(&self, path: &str, expected: &str) -> bool {
        read_source_bytes(&self.workspace.files, path)
            .is_ok_and(|bytes| revision_hash(&bytes) == expected)
    }

    pub(crate) fn list_analyses(&self, path: &str) -> M2Result<Vec<Value>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT j.id FROM m2_analyses a JOIN jobs j ON j.id=a.id
            JOIN sources s ON s.id=j.source_id WHERE s.location=?1 ORDER BY j.rowid DESC",
            )
            .map_err(db)?;
        let ids = statement
            .query_map([path], |row| row.get::<_, String>(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        ids.iter().map(|id| self.analysis(id)).collect()
    }

    pub(crate) fn analysis(&self, id: &str) -> M2Result<Value> {
        let (path, revision, state, error, data): (String, String, String, Option<String>, String) =
            self.connection
                .query_row(
                    "SELECT s.location,j.source_revision,j.state,a.error,a.items FROM m2_analyses a
                JOIN jobs j ON j.id=a.id JOIN sources s ON s.id=j.source_id WHERE a.id=?1",
                    [id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .map_err(db)?;
        let state = if state == "succeeded" && !self.source_is_current(&path, &revision) {
            "stale"
        } else {
            &state
        };
        let error =
            error.or_else(|| (state == "failed").then(|| "上一次处理未完成，可重新整理".into()));
        let mut items = parse(&data)?;
        if state == "stale" {
            for item in items.as_array_mut().ok_or("整理项目数据无法读取")? {
                item["source"] = self.verify_evidence(&item["source"])?;
            }
        }
        Ok(
            json!({"id":id,"path":path,"revision":revision,"status":state,"error":error,"items":items}),
        )
    }

    pub(crate) fn start_analysis(
        &mut self,
        path: &str,
        revision: &str,
        config: &ModelConfig,
    ) -> M2Result<AnalysisRequest> {
        let document = self.load_document(path, revision)?;
        if document.body.trim().is_empty() || document.body.len() > SOURCE_LIMIT {
            return Err("原文为空或超出本次整理的资料限额（48 KiB）".into());
        }
        let key = format!(
            "{}:{}:analyze:{}:{ANALYSIS_RULE}",
            document.id,
            revision,
            revision_hash(json!([config.0, config.1]).to_string().as_bytes())
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute("INSERT OR IGNORE INTO jobs VALUES(lower(hex(randomblob(16))),?1,'analyze',?2,?3,'queued',0,0)",
            params![key,document.id,revision]).map_err(db)?;
        let (id, state): (String, String) = tx
            .query_row(
                "SELECT id,state FROM jobs WHERE operation_key=?1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(db)?;
        tx.execute("INSERT OR IGNORE INTO m2_analyses(id) VALUES(?1)", [&id])
            .map_err(db)?;
        if state == "succeeded" || state == "running" {
            tx.commit().map_err(db)?;
            return self.analysis(&id).map(AnalysisRequest::Cached);
        }
        tx.execute(
            "UPDATE jobs SET state='running', attempts=attempts+1 WHERE id=?1",
            [&id],
        )
        .map_err(db)?;
        tx.execute("UPDATE m2_analyses SET error=NULL WHERE id=?1", [&id])
            .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(AnalysisRequest::Generate { id, document })
    }

    pub(crate) fn finish_analysis(
        &mut self,
        id: &str,
        document: &Document,
        config: &ModelConfig,
        result: M2Result<Value>,
    ) -> M2Result<Value> {
        let current = self.model_config()?.as_ref() == Some(config)
            && self.source_is_current(&document.path, &document.revision);
        let (state, error, items) = match result.and_then(|data| analysis_items(document, id, data))
        {
            Ok(items) if current => ("succeeded", None, items),
            Ok(items) => (
                "stale",
                Some("原文或模型配置已改变，已保留整理结果供核对".into()),
                items,
            ),
            Err(detail) => ("failed", Some(detail), vec![]),
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute(
            "UPDATE m2_analyses SET error=?2, items=?3 WHERE id=?1",
            params![id, error, json!(items).to_string()],
        )
        .map_err(db)?;
        tx.execute(
            "UPDATE jobs SET state=?2 WHERE id=?1 AND state='running'",
            params![id, state],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        self.analysis(id)
    }

    pub(crate) fn analysis_item(&self, key: &str) -> M2Result<Value> {
        let (analysis_id, _) = key.split_once(':').ok_or("整理条目标识无效")?;
        let analysis = self.analysis(analysis_id)?;
        if analysis["status"] != "succeeded" {
            return Err("整理结果已失效，请重新整理后再采用".into());
        }
        analysis["items"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == key))
            .cloned()
            .ok_or_else(|| "找不到整理条目".into())
    }

    pub(crate) fn verify_evidence(&self, evidence: &Value) -> M2Result<Value> {
        let path = required(evidence, "path")?;
        let revision = required(evidence, "revision")?;
        let mut result = evidence.clone();
        if result["isCapture"].is_null() {
            result["isCapture"] = json!(self.is_capture_path(path));
        }
        match read_source_bytes(&self.workspace.files, path) {
            Ok(bytes) => {
                result["available"] = json!(true);
                result["changed"] = json!(revision_hash(&bytes) != revision);
            }
            Err(error)
                if error.kind() == io::ErrorKind::NotFound && physical_path(path) == path =>
            {
                let mut statement = self.connection.prepare(
                    "SELECT relative_path FROM search_documents WHERE revision=?1 AND relative_path<>?2 LIMIT 2").map_err(db)?;
                let matches = statement
                    .query_map(params![revision, path], |row| row.get::<_, String>(0))
                    .map_err(db)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db)?;
                if let [moved] = matches.as_slice()
                    && self.source_is_current(moved, revision)
                {
                    result["movedFrom"] = json!(path);
                    result["path"] = json!(moved);
                    result["available"] = json!(true);
                    result["changed"] = json!(false);
                    return Ok(result);
                }
                result["available"] = json!(false);
                result["excerpt"] = json!("");
            }
            Err(_) => {
                result["available"] = json!(false);
                result["excerpt"] = json!("");
            }
        };
        Ok(result)
    }
}

fn event_date(connection: &Connection, body: &str) -> M2Result<Option<String>> {
    let mut lines = body.trim_start_matches('\u{feff}').lines();
    if lines.next() != Some("---") {
        return Ok(None);
    }
    let date = lines.take_while(|line| *line != "---").find_map(|line| {
        let (key, value) = line.split_once(':')?;
        matches!(key, "event_date" | "published_date")
            .then(|| value.trim().trim_matches(['\'', '"']))
    });
    match date {
        Some(date) => connection
            .query_row("SELECT date(?1,'+0 days') IS ?1", [date], |row| {
                row.get::<_, bool>(0)
            })
            .map(|valid| valid.then(|| date.to_owned()))
            .map_err(db),
        None => Ok(None),
    }
}

fn line_number(value: &Value, key: &str) -> M2Result<u32> {
    value[key]
        .as_u64()
        .and_then(|line| u32::try_from(line).ok())
        .ok_or_else(|| "模型输出缺少有效原文行号".into())
}

fn analysis_items(document: &Document, id: &str, response: Value) -> M2Result<Vec<Value>> {
    let generated = response["items"]
        .as_array()
        .filter(|items| items.len() <= ITEM_LIMIT)
        .ok_or("模型未返回有效的纪要和候选列表")?;
    let mut result = Vec::new();
    let mut fence = None;
    for (index, line) in document.body.lines().enumerate() {
        let line = line.trim_start();
        let marker = line
            .chars()
            .next()
            .filter(|marker| matches!(marker, '~' | '\x60'));
        if let Some(marker) = marker
            && line.chars().take(3).all(|ch| ch == marker)
            && line.len() >= 3
        {
            if fence == Some(marker) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if let Some(task) = line
            .strip_prefix("- [ ] ")
            .or_else(|| line.strip_prefix("* [ ] "))
            && !task.trim().is_empty()
            && task.len() <= EXCERPT_LIMIT
        {
            let line = index as u32 + 1;
            let source = document.evidence(line, line, false)?;
            result.push(
                json!({"id":format!("{id}:{}",result.len()),"kind":"action","text":task,
                "source":source,"assignee":null,"due":null}),
            );
        }
    }
    for item in generated {
        let kind = required(item, "kind")?;
        let text = required(item, "text")?;
        if !matches!(kind, "discussion" | "decision" | "action" | "open" | "wiki")
            || text.len() > EXCERPT_LIMIT
        {
            return Err("模型输出含无效纪要类别或超长条目".into());
        }
        let first = line_number(item, "startLine")?;
        let last = line_number(item, "endLine")?;
        let source = document.evidence(first, last, false)?;
        let excerpt = source["excerpt"].as_str().ok_or("原文证据无效")?;
        let supported = |key| {
            item[key]
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty() && excerpt.contains(value))
        };
        if kind == "action" {
            let mut merged = false;
            for entry in &mut result {
                if entry["kind"] == "action"
                    && entry["source"]["startLine"]
                        .as_u64()
                        .is_some_and(|n| first as u64 <= n && n <= last as u64)
                {
                    let entry_excerpt = entry["source"]["excerpt"].as_str().unwrap_or("");
                    let next_assignee = entry["assignee"]
                        .is_null()
                        .then(|| supported("assignee"))
                        .flatten()
                        .filter(|assignee| entry_excerpt.contains(assignee));
                    let next_due = entry["due"]
                        .is_null()
                        .then(|| supported("due"))
                        .flatten()
                        .filter(|due| entry_excerpt.contains(due));
                    if let Some(assignee) = next_assignee {
                        entry["assignee"] = json!(assignee);
                    }
                    if let Some(due) = next_due {
                        entry["due"] = json!(due);
                    }
                    merged = true;
                }
            }
            if merged {
                continue;
            }
        }
        if result
            .iter()
            .any(|entry| entry["kind"] == kind && entry["text"] == text)
        {
            continue;
        }
        result.push(
            json!({"id":format!("{id}:{}",result.len()),"kind":kind,"text":text,"source":source,
            "assignee":supported("assignee"),"due":supported("due")}),
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::TestDir;
    use std::fs;

    #[test]
    fn analysis_keeps_missing_fields_and_reuses_the_job_for_the_same_input() {
        let dir = TestDir::new();
        let note = "# 例会\n- [ ] 核对会议附件\n- [ ] 李四 周五前核对备份步骤\n\x60\x60\x60\n- [ ] 示例任务\n\x60\x60\x60\n";
        fs::write(dir.0.join("note.md"), note).unwrap();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".state").unwrap();
        let model = storage
            .set_model_config(
                "http://127.0.0.1/v1/chat/completions".into(),
                "fixture".into(),
                true,
            )
            .unwrap();
        let revision = revision_hash(note.as_bytes());
        let AnalysisRequest::Generate { id, document } = storage
            .start_analysis("note.md", &revision, &model)
            .unwrap()
        else {
            panic!("expected a new job");
        };
        let result = storage.finish_analysis(&id,&document,&model,Ok(json!({"items":[
            {"kind":"action","text":"核对附件","startLine":2,"endLine":2,"assignee":"凭空指派"},
            {"kind":"action","text":"核对备份步骤","startLine":3,"endLine":3,"assignee":"李四","due":"周五前"},
            {"kind":"discussion","text":"讨论会议事项","startLine":1,"endLine":1}
        ]}))).unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 3);
        assert_eq!(result["items"][0]["assignee"], Value::Null);
        assert_eq!(result["items"][0]["due"], Value::Null);
        assert_eq!(
            result["items"][0]["source"]["excerpt"],
            "- [ ] 核对会议附件"
        );
        assert_eq!(result["items"][1]["text"], "李四 周五前核对备份步骤");
        assert_eq!(result["items"][1]["assignee"], "李四");
        assert_eq!(result["items"][1]["due"], "周五前");
        assert!(matches!(
            storage
                .start_analysis("note.md", &revision, &model)
                .unwrap(),
            AnalysisRequest::Cached(_)
        ));
        fs::write(dir.0.join("note.md"), "新版本").unwrap();
        assert_eq!(storage.analysis(&id).unwrap()["status"], "stale");
        assert!(storage.analysis_item(&format!("{id}:0")).is_err());
    }
}
