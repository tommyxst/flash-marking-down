use std::collections::HashSet;

use serde_json::{Value, json};

use super::m2::{M2Result, ModelConfig, db, parse, required};
use super::*;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS disagreements (
    id TEXT PRIMARY KEY,
    question TEXT NOT NULL,
    relation TEXT NOT NULL,
    status TEXT NOT NULL,
    judgment TEXT NOT NULL DEFAULT '',
    reason TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS disagreement_members (
    disagreement_id TEXT NOT NULL REFERENCES disagreements(id) ON DELETE CASCADE,
    member_order INTEGER NOT NULL,
    claim_id TEXT NOT NULL,
    member_text TEXT NOT NULL,
    evidence_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    file_identity TEXT NOT NULL,
    path TEXT NOT NULL,
    revision TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    evidence_json TEXT NOT NULL,
    PRIMARY KEY (disagreement_id, member_order)
);
CREATE INDEX IF NOT EXISTS idx_disagreement_members_claim
    ON disagreement_members(claim_id);
CREATE INDEX IF NOT EXISTS idx_disagreement_members_source_rev
    ON disagreement_members(source_id, revision);
CREATE INDEX IF NOT EXISTS idx_disagreement_members_file_rev
    ON disagreement_members(file_identity, revision);
CREATE INDEX IF NOT EXISTS idx_disagreement_members_evidence
    ON disagreement_members(evidence_id);
CREATE TABLE IF NOT EXISTS disagreement_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    disagreement_id TEXT NOT NULL REFERENCES disagreements(id) ON DELETE CASCADE,
    recorded_at TEXT NOT NULL,
    actor TEXT NOT NULL,
    status TEXT NOT NULL,
    judgment TEXT NOT NULL,
    reason TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_disagreement_history_disagreement
    ON disagreement_history(disagreement_id, id);";

pub(super) fn migrate_legacy_disagreements(tx: &rusqlite::Transaction<'_>) -> M2Result<()> {
    let legacy_exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'm2_disagreements')",
            [],
            |row| row.get(0),
        )
        .map_err(db)?;
    if !legacy_exists {
        return Ok(());
    }
    let rows: Vec<(String, String)> = {
        let mut statement = tx
            .prepare("SELECT id, data FROM m2_disagreements ORDER BY rowid ASC")
            .map_err(db)?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?
    };
    for (row_id, raw) in rows {
        let data = parse(&raw)?;
        let id = data["id"].as_str().unwrap_or(&row_id);
        let question = data["question"]
            .as_str()
            .ok_or("旧分歧记录缺少问题字段，迁移中止")?;
        let relation = data["relation"]
            .as_str()
            .ok_or("旧分歧记录缺少关系类型，迁移中止")?;
        let status = data["status"]
            .as_str()
            .ok_or("旧分歧记录缺少状态字段，迁移中止")?;
        let judgment = data["judgment"].as_str().unwrap_or("");
        let reason = data["reason"].as_str().unwrap_or("");
        tx.execute(
            "INSERT INTO disagreements (id, question, relation, status, judgment, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, question, relation, status, judgment, reason],
        )
        .map_err(db)?;
        for (order, member) in entries(&data, "members")?.iter().enumerate() {
            let source = &member["source"];
            let claim = claim_id(member)?;
            let member_text = member["text"].as_str().unwrap_or("");
            let evidence_id = source["id"].as_str().unwrap_or("");
            let source_id = source["sourceId"].as_str().unwrap_or("");
            let file_identity = source["fileIdentity"].as_str().unwrap_or("");
            let path = source["path"].as_str().unwrap_or("");
            let revision = source["revision"].as_str().unwrap_or("");
            let start_line = source["startLine"].as_i64().unwrap_or(0);
            let end_line = source["endLine"].as_i64().unwrap_or(0);
            tx.execute(
                "INSERT INTO disagreement_members
                 (disagreement_id, member_order, claim_id, member_text, evidence_id, source_id,
                  file_identity, path, revision, start_line, end_line, evidence_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    order as i64,
                    claim,
                    member_text,
                    evidence_id,
                    source_id,
                    file_identity,
                    path,
                    revision,
                    start_line,
                    end_line,
                    source.to_string(),
                ],
            )
            .map_err(db)?;
        }
        if let Some(history) = data["history"].as_array() {
            for item in history {
                tx.execute(
                    "INSERT INTO disagreement_history
                     (disagreement_id, recorded_at, actor, status, judgment, reason)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        id,
                        item["at"].as_str().unwrap_or(""),
                        item["actor"].as_str().unwrap_or("user"),
                        item["status"].as_str().unwrap_or(status),
                        item["judgment"].as_str().unwrap_or(""),
                        item["reason"].as_str().unwrap_or(""),
                    ],
                )
                .map_err(db)?;
            }
        }
    }
    tx.execute("DROP TABLE m2_disagreements", []).map_err(db)?;
    Ok(())
}

const QUERY_LIMIT: usize = 4096;
const MAX_PRIMARY_SOURCES: usize = 6;
const MAX_CANDIDATES: u32 = 200;
const MAX_CONTEXT_BYTES: usize = 32 * 1024;
const MAX_CLAIMS: usize = 20;
const MAX_MEMBER_BYTES: usize = 4800;

pub(crate) struct DateScope {
    from: Option<String>,
    to: Option<String>,
    basis: String,
}

impl DateScope {
    pub(crate) fn new(from: Option<String>, to: Option<String>, basis: String) -> M2Result<Self> {
        if !matches!(basis.as_str(), "recorded" | "event")
            || matches!((&from, &to), (Some(from), Some(to)) if from > to)
        {
            return Err("日期范围无效".into());
        }
        Ok(Self { from, to, basis })
    }

    fn contains(&self, connection: &Connection, evidence: &Value) -> M2Result<bool> {
        if self.from.is_none() && self.to.is_none() {
            return Ok(true);
        }
        let date = match self.basis.as_str() {
            "recorded" => evidence["date"]
                .as_str()
                .map(|utc| {
                    connection
                        .query_row("SELECT date(?1,'localtime')", [utc], |row| {
                            row.get::<_, String>(0)
                        })
                        .map_err(db)
                })
                .transpose()?,
            "event" => evidence["eventDate"].as_str().map(str::to_owned),
            _ => return Err("日期依据无效".into()),
        };
        Ok(date.is_some_and(|date| {
            self.from.as_ref().is_none_or(|from| &date >= from)
                && self.to.as_ref().is_none_or(|to| &date <= to)
        }))
    }

    fn validate(&self, connection: &Connection) -> M2Result<()> {
        for date in self.from.iter().chain(&self.to) {
            let valid: bool = connection
                .query_row("SELECT date(?1,'+0 days') IS ?1", [date], |row| row.get(0))
                .map_err(db)?;
            if !valid {
                return Err("日期必须有效且使用 YYYY-MM-DD".into());
            }
        }
        Ok(())
    }
}

fn entries<'a>(value: &'a Value, key: &str) -> M2Result<&'a Vec<Value>> {
    value[key]
        .as_array()
        .ok_or_else(|| format!("缺少有效列表 {key}"))
}

fn overlaps(a: &Value, b: &Value) -> bool {
    a["id"] == b["id"]
        || (a["sourceId"] == b["sourceId"]
            || a["fileIdentity"]
                .as_str()
                .is_some_and(|id| b["fileIdentity"] == id))
            && a["revision"] == b["revision"]
            && matches!((a["startLine"].as_u64(),a["endLine"].as_u64(),b["startLine"].as_u64(),b["endLine"].as_u64()),
            (Some(a1),Some(a2),Some(b1),Some(b2)) if a1 <= b2 && b1 <= a2)
}

fn claim_id(member: &Value) -> M2Result<String> {
    if let Some(id) = member["claimId"].as_str() {
        return Ok(id.to_owned());
    }
    let source = required(&member["source"], "id")?;
    let text = required(member, "text")?;
    Ok(revision_hash(json!([source, text]).to_string().as_bytes()))
}

fn covered(evidence: &Value) -> bool {
    evidence["available"] == true && evidence["changed"] == false
}

impl Storage {
    fn check_disagreement_store_integrity(&self) -> M2Result<()> {
        let corrupted: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM disagreements d
                    WHERE d.relation NOT IN ('contradiction', 'conditions', 'historical')
                       OR d.status NOT IN ('open', 'decided', 'resolved', 'superseded')
                       OR trim(d.question) = ''
                       OR (SELECT count(*) FROM disagreement_members m WHERE m.disagreement_id = d.id) < 2
                       OR EXISTS (
                           SELECT 1 FROM disagreement_members m
                           WHERE m.disagreement_id = d.id AND trim(m.evidence_id) = ''
                       )
                )",
                [],
                |row| row.get(0),
            )
            .map_err(db)?;
        if corrupted {
            return Err("分歧关系无法核对，已停止生成和复制。仍可使用本地搜索".into());
        }
        Ok(())
    }

    fn load_disagreement_by_id(&self, id: &str) -> M2Result<Value> {
        let (question, relation, status, judgment, reason): (
            String,
            String,
            String,
            String,
            String,
        ) = self
            .connection
            .query_row(
                "SELECT question, relation, status, judgment, reason FROM disagreements WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .map_err(db)?;
        if !matches!(
            relation.as_str(),
            "contradiction" | "conditions" | "historical"
        ) || !matches!(
            status.as_str(),
            "open" | "decided" | "resolved" | "superseded"
        ) || question.trim().is_empty()
        {
            return Err("分歧关系无法核对，已停止生成和复制。仍可使用本地搜索".into());
        }
        let members: Vec<Value> = {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT claim_id, member_text, evidence_json
                     FROM disagreement_members
                     WHERE disagreement_id = ?1
                     ORDER BY member_order ASC",
                )
                .map_err(db)?;
            statement
                .query_map([id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(db)?
                .map(|row| {
                    let (claim, text, evidence_raw) = row.map_err(db)?;
                    let source = parse(&evidence_raw)?;
                    if source["id"].as_str().is_none() {
                        return Err("分歧关系无法核对，已停止生成和复制。仍可使用本地搜索".into());
                    }
                    Ok(json!({ "claimId": claim, "text": text, "source": source }))
                })
                .collect::<M2Result<Vec<_>>>()?
        };
        if members.len() < 2 {
            return Err("分歧关系无法核对，已停止生成和复制。仍可使用本地搜索".into());
        }
        for (index, member) in members.iter().enumerate() {
            if members[..index]
                .iter()
                .any(|other| overlaps(&other["source"], &member["source"]))
            {
                return Err("分歧成员指向重复证据，已停止生成和复制".into());
            }
        }
        let history: Vec<Value> = {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT recorded_at, actor, status, judgment, reason
                     FROM disagreement_history
                     WHERE disagreement_id = ?1
                     ORDER BY id ASC",
                )
                .map_err(db)?;
            statement
                .query_map([id], |row| {
                    Ok(json!({
                        "at": row.get::<_, String>(0)?,
                        "actor": row.get::<_, String>(1)?,
                        "status": row.get::<_, String>(2)?,
                        "judgment": row.get::<_, String>(3)?,
                        "reason": row.get::<_, String>(4)?,
                    }))
                })
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?
        };
        Ok(json!({
            "id": id,
            "question": question,
            "relation": relation,
            "members": members,
            "status": status,
            "judgment": judgment,
            "reason": reason,
            "history": history,
        }))
    }

    fn disagreement_view(&self, mut value: Value, scope: &DateScope) -> M2Result<Value> {
        let mut missing_source = false;
        let mut changed_source = false;
        for member in value["members"]
            .as_array_mut()
            .ok_or("分歧资料关系无法核对")?
        {
            member["claimId"] = json!(claim_id(member)?);
            let mut evidence = self.verify_evidence(&member["source"])?;
            evidence["outsideRange"] = json!(!scope.contains(&self.connection, &evidence)?);
            if evidence["available"] != true {
                missing_source = true;
                member["text"] = json!("");
                evidence["excerpt"] = json!("");
            } else if evidence["changed"] == true {
                changed_source = true;
            }
            member["source"] = evidence;
        }
        if missing_source {
            value["judgment"] = Value::Null;
            value["reason"] = Value::Null;
            value["history"] = json!([]);
            value["judgmentNeedsSources"] = json!(true);
        } else if changed_source {
            value["sourcesChanged"] = json!(true);
        }
        Ok(value)
    }

    pub(crate) fn list_disagreements(&self) -> M2Result<Vec<Value>> {
        let scope = DateScope::new(None, None, "recorded".into())?;
        let ids: Vec<String> = {
            let mut statement = self
                .connection
                .prepare("SELECT id FROM disagreements ORDER BY rowid DESC")
                .map_err(db)?;
            statement
                .query_map([], |row| row.get(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?
        };
        ids.into_iter()
            .map(|id| {
                let data = self.load_disagreement_by_id(&id)?;
                self.disagreement_view(data, &scope)
            })
            .collect()
    }

    pub(crate) fn create_disagreement(
        &mut self,
        question: &str,
        relation: &str,
        members: Vec<Value>,
    ) -> M2Result<Value> {
        let question = question.trim();
        if question.is_empty()
            || question.len() > QUERY_LIMIT
            || members.len() != 2
            || !matches!(relation, "contradiction" | "conditions" | "historical")
        {
            return Err("请填写分歧问题、关系和两方原文依据".into());
        }
        let mut refs = Vec::new();
        for member in members {
            let excerpt = required(&member, "excerpt")?;
            if excerpt.len() > MAX_MEMBER_BYTES {
                return Err("分歧片段过长".into());
            }
            let document =
                self.load_document(required(&member, "path")?, required(&member, "revision")?)?;
            let evidence = document.locate(excerpt)?;
            let mut member = json!({"text":excerpt,"source":evidence});
            member["claimId"] = json!(claim_id(&member)?);
            refs.push(member);
        }
        if overlaps(&refs[0]["source"], &refs[1]["source"]) {
            return Err("两方需要引用不同的原文片段".into());
        }
        let id = self.random_m2_id()?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute(
            "INSERT INTO disagreements (id, question, relation, status, judgment, reason)
             VALUES (?1, ?2, ?3, 'open', '', '')",
            params![id, question, relation],
        )
        .map_err(db)?;
        for (order, member) in refs.iter().enumerate() {
            let source = &member["source"];
            tx.execute(
                "INSERT INTO disagreement_members
                 (disagreement_id, member_order, claim_id, member_text, evidence_id, source_id,
                  file_identity, path, revision, start_line, end_line, evidence_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    order as i64,
                    member["claimId"].as_str().unwrap_or(""),
                    member["text"].as_str().unwrap_or(""),
                    source["id"].as_str().unwrap_or(""),
                    source["sourceId"].as_str().unwrap_or(""),
                    source["fileIdentity"].as_str().unwrap_or(""),
                    source["path"].as_str().unwrap_or(""),
                    source["revision"].as_str().unwrap_or(""),
                    source["startLine"].as_i64().unwrap_or(0),
                    source["endLine"].as_i64().unwrap_or(0),
                    source.to_string(),
                ],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        Ok(json!({
            "id": id,
            "question": question,
            "relation": relation,
            "members": refs,
            "status": "open",
            "judgment": "",
            "reason": "",
            "history": []
        }))
    }

    pub(crate) fn review_disagreement(
        &mut self,
        id: &str,
        status: &str,
        judgment: &str,
        reason: &str,
    ) -> M2Result<Value> {
        let judgment = judgment.trim();
        let reason = reason.trim();
        if !matches!(status, "open" | "decided" | "resolved" | "superseded")
            || status != "open" && (judgment.is_empty() || reason.is_empty())
            || judgment.len() + reason.len() > QUERY_LIMIT
        {
            return Err("请填写本次判断和依据，保留原有各方观点".into());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let updated = tx
            .execute(
                "UPDATE disagreements SET status = ?2, judgment = ?3, reason = ?4 WHERE id = ?1",
                params![id, status, judgment, reason],
            )
            .map_err(db)?;
        if updated != 1 {
            return Err("找不到对应的分歧记录".into());
        }
        let at: String = tx
            .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |r| {
                r.get(0)
            })
            .map_err(db)?;
        tx.execute(
            "INSERT INTO disagreement_history
             (disagreement_id, recorded_at, actor, status, judgment, reason)
             VALUES (?1, ?2, 'user', ?3, ?4, ?5)",
            params![id, at, status, judgment, reason],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        let result = self.load_disagreement_by_id(id)?;
        self.disagreement_view(result, &DateScope::new(None, None, "recorded".into())?)
    }

    fn relevant_disagreements(
        &self,
        query: &str,
        keyword: &str,
        sources: &[Value],
        scope: &DateScope,
        anchor_claim: Option<&str>,
    ) -> M2Result<Vec<Value>> {
        self.check_disagreement_store_integrity()?;
        let mut matched_ids = HashSet::new();
        if let Some(claim) = anchor_claim {
            let mut statement = self
                .connection
                .prepare("SELECT disagreement_id FROM disagreement_members WHERE claim_id = ?1")
                .map_err(db)?;
            let ids = statement
                .query_map([claim], |row| row.get::<_, String>(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            if ids.is_empty() {
                return Err("所选主张已不可用，请刷新分歧记录".into());
            }
            matched_ids.extend(ids);
        }
        {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT id FROM disagreements
                     WHERE instr(?1, question) > 0
                        OR (?2 <> '' AND instr(question, ?2) > 0)",
                )
                .map_err(db)?;
            let ids = statement
                .query_map(params![query, keyword], |row| row.get::<_, String>(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            matched_ids.extend(ids);
        }
        if !sources.is_empty() {
            let mut by_evidence = self
                .connection
                .prepare("SELECT disagreement_id FROM disagreement_members WHERE evidence_id = ?1")
                .map_err(db)?;
            let mut by_source = self
                .connection
                .prepare(
                    "SELECT disagreement_id FROM disagreement_members
                     WHERE source_id = ?1 AND revision = ?2
                       AND start_line <= ?4 AND ?3 <= end_line",
                )
                .map_err(db)?;
            let mut by_file = self
                .connection
                .prepare(
                    "SELECT disagreement_id FROM disagreement_members
                     WHERE file_identity = ?1 AND revision = ?2
                       AND start_line <= ?4 AND ?3 <= end_line",
                )
                .map_err(db)?;
            for source in sources {
                if let Some(evidence_id) = source["id"].as_str() {
                    for id in by_evidence
                        .query_map([evidence_id], |row| row.get::<_, String>(0))
                        .map_err(db)?
                    {
                        matched_ids.insert(id.map_err(db)?);
                    }
                }
                if let (Some(revision), Some(start), Some(end)) = (
                    source["revision"].as_str(),
                    source["startLine"].as_i64(),
                    source["endLine"].as_i64(),
                ) {
                    if let Some(source_id) = source["sourceId"].as_str() {
                        for id in by_source
                            .query_map(params![source_id, revision, start, end], |row| {
                                row.get::<_, String>(0)
                            })
                            .map_err(db)?
                        {
                            matched_ids.insert(id.map_err(db)?);
                        }
                    }
                    if let Some(file_id) = source["fileIdentity"]
                        .as_str()
                        .filter(|value| !value.is_empty())
                    {
                        for id in by_file
                            .query_map(params![file_id, revision, start, end], |row| {
                                row.get::<_, String>(0)
                            })
                            .map_err(db)?
                        {
                            matched_ids.insert(id.map_err(db)?);
                        }
                    }
                }
            }
        }
        if matched_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ordered_ids: Vec<String> = {
            let mut statement = self
                .connection
                .prepare(
                    "SELECT d.id FROM disagreements d
                     JOIN json_each(?1) j ON d.id = j.value
                     ORDER BY d.rowid DESC",
                )
                .map_err(db)?;
            let id_list: Vec<&String> = matched_ids.iter().collect();
            statement
                .query_map([json!(id_list).to_string()], |row| row.get(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?
        };
        let mut related = Vec::with_capacity(ordered_ids.len());
        for id in ordered_ids {
            let data = self.load_disagreement_by_id(&id)?;
            related.push(self.disagreement_view(data, scope)?);
        }
        Ok(related)
    }

    pub(crate) fn prepare_answer(
        &mut self,
        query: &str,
        keyword: &str,
        scope: DateScope,
        output_kind: &str,
        anchor_claim: Option<&str>,
    ) -> M2Result<Value> {
        let (query, keyword) = (query.trim(), keyword.trim());
        if query.is_empty() || query.len() > QUERY_LIMIT || keyword.len() > QUERY_LIMIT {
            return Err("请填写有效的问题和关键词".into());
        }
        if !matches!(output_kind, "answer" | "summary" | "plan") {
            return Err("输出类型无效".into());
        }
        scope.validate(&self.connection)?;
        let config = self
            .model_config()?
            .filter(|config| config.2)
            .ok_or("请先配置并启用模型")?;
        let terms = search_terms(query, keyword);
        // A user supplied keyword uses the same literal matching as local search.
        let variants = json!(terms).to_string();
        let fts_expr: Option<String> = terms
            .iter()
            .map(|term| search::fts_trigram_phrase(term))
            .collect::<Option<Vec<_>>>()
            .filter(|list| !list.is_empty())
            .map(|list| list.join(" OR "));
        let hits = {
            if let Some(fts_query) = fts_expr {
                let mut statement = self
                    .connection
                    .prepare(
                        "WITH matched AS MATERIALIZED (
                       SELECT d.relative_path, d.revision, d.captured_at,
                         COALESCE((SELECT sum(length(value)) FROM json_each(?1)
                           WHERE instr(lower(d.body), lower(value)) > 0
                              OR instr(lower(d.relative_path), lower(value)) > 0), 0) AS score
                       FROM search_documents d
                       WHERE d.rowid IN (SELECT rowid FROM search_documents_fts WHERE search_documents_fts MATCH ?6)
                         AND (?3='event' OR ((?4 IS NULL OR date(d.captured_at,'localtime')>=?4)
                           AND (?5 IS NULL OR date(d.captured_at,'localtime')<=?5)))
                     )
                     SELECT relative_path, revision FROM matched WHERE score > 0
                     ORDER BY score DESC, captured_at DESC, relative_path LIMIT ?2",
                    )
                    .map_err(db)?;
                statement
                    .query_map(
                        params![
                            variants,
                            MAX_CANDIDATES,
                            scope.basis,
                            scope.from,
                            scope.to,
                            fts_query
                        ],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .map_err(db)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db)?
            } else {
                let mut statement = self
                    .connection
                    .prepare(
                        "WITH matched AS MATERIALIZED (
                       SELECT d.relative_path, d.revision, d.captured_at,
                         COALESCE((SELECT sum(length(value)) FROM json_each(?1)
                           WHERE instr(lower(d.body), lower(value)) > 0
                              OR instr(lower(d.relative_path), lower(value)) > 0), 0) AS score
                       FROM search_documents d
                       WHERE ?3='event' OR ((?4 IS NULL OR date(d.captured_at,'localtime')>=?4)
                         AND (?5 IS NULL OR date(d.captured_at,'localtime')<=?5))
                     )
                     SELECT relative_path, revision FROM matched WHERE score > 0
                     ORDER BY score DESC, captured_at DESC, relative_path LIMIT ?2",
                    )
                    .map_err(db)?;
                statement
                    .query_map(
                        params![variants, MAX_CANDIDATES, scope.basis, scope.from, scope.to],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .map_err(db)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db)?
            }
        };
        let mut sources = Vec::new();
        let mut skipped = 0;
        for (path, revision) in hits {
            if sources.len() >= MAX_PRIMARY_SOURCES {
                break;
            }
            let document = match self.load_document(&path, &revision) {
                Ok(document) => document,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let candidate = terms.iter().find(|term| {
                document
                    .body
                    .to_ascii_lowercase()
                    .contains(&term.to_ascii_lowercase())
                    || path
                        .to_ascii_lowercase()
                        .contains(&term.to_ascii_lowercase())
            });
            let Some(term) = candidate else {
                continue;
            };
            if let Ok(evidence) = document.search_evidence(term)
                && scope.contains(&self.connection, &evidence)?
            {
                sources.push(evidence);
            }
        }
        let initial = sources.clone();
        let disagreements =
            self.relevant_disagreements(query, keyword, &initial, &scope, anchor_claim)?;
        for item in &disagreements {
            for member in entries(item, "members")? {
                let source = &member["source"];
                if covered(source)
                    && !sources
                        .iter()
                        .any(|existing| existing["id"] == source["id"])
                {
                    sources.push(source.clone());
                }
            }
        }
        if sources.is_empty() {
            return Err("找不到可引用的原文，请调整关键词或日期，或在外部编辑后重建索引".into());
        }
        if json!(sources).to_string().len() > MAX_CONTEXT_BYTES {
            return Err("完整依据超出发送范围，请缩小问题或日期范围".into());
        }
        let id = self.random_m2_id()?;
        let data = json!({"id":id,"endpoint":config.0,"model":config.1,"query":query,"keyword":keyword,"outputKind":output_kind,
            "dateBasis":scope.basis,"fromDate":scope.from,"toDate":scope.to,"anchorClaimId":anchor_claim,
            "sources":sources,"initial":initial,"disagreements":disagreements,"skipped":skipped});
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        tx.execute("DELETE FROM m2_preparations", []).map_err(db)?;
        tx.execute(
            "INSERT INTO m2_preparations VALUES(?1,?2)",
            params![id, data.to_string()],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(data)
    }

    pub(crate) fn start_answer(&mut self, id: &str, config: &ModelConfig) -> M2Result<Value> {
        let data: String = self
            .connection
            .query_row("SELECT data FROM m2_preparations WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db)?
            .ok_or("请先重新准备资料")?;
        let data = parse(&data)?;
        self.check_answer_inputs(&data, config)?;
        self.connection
            .execute("DELETE FROM m2_preparations WHERE id=?1", [id])
            .map_err(db)?;
        Ok(data)
    }

    pub(crate) fn cancel_answer_preparation(&self, id: &str) -> M2Result<()> {
        self.connection
            .execute("DELETE FROM m2_preparations WHERE id=?1", [id])
            .map_err(db)?;
        Ok(())
    }

    fn check_answer_inputs(&self, data: &Value, config: &ModelConfig) -> M2Result<()> {
        if self.model_config()?.as_ref() != Some(config)
            || data["endpoint"] != config.0
            || data["model"] != config.1
        {
            return Err("模型配置已改变，请重新确认资料范围".into());
        }
        for evidence in entries(data, "sources")? {
            let current = self.verify_evidence(evidence)?;
            if !covered(&current) || current["path"] != evidence["path"] {
                return Err("来源已变化，请重新准备资料".into());
            }
        }
        let current = self.answer_disagreements(data)?;
        if current != data["disagreements"] {
            return Err("分歧关系或来源已更新，请重新准备资料".into());
        }
        Ok(())
    }

    fn answer_scope(data: &Value) -> M2Result<DateScope> {
        DateScope::new(
            data["fromDate"].as_str().map(str::to_owned),
            data["toDate"].as_str().map(str::to_owned),
            required(data, "dateBasis")?.to_owned(),
        )
    }

    fn answer_disagreements(&self, data: &Value) -> M2Result<Value> {
        let scope = Self::answer_scope(data)?;
        // Relations use the model's candidate context, before relation members expand that context.
        let related = self.relevant_disagreements(
            required(data, "query")?,
            data["keyword"].as_str().ok_or("查询范围不可用")?,
            entries(data, "initial")?,
            &scope,
            data["anchorClaimId"].as_str(),
        )?;
        Ok(json!(related))
    }

    pub(crate) fn finish_answer(
        &mut self,
        mut data: Value,
        config: &ModelConfig,
        result: Value,
    ) -> M2Result<Value> {
        self.check_answer_inputs(&data, config)?;
        let claims = entries(&result, "claims")?;
        if claims.is_empty() || claims.len() > MAX_CLAIMS {
            return Err("模型未给出带依据的结论".into());
        }
        let sources = entries(&data, "sources")?;
        let mut used_claims = Vec::new();
        let mut text = String::new();
        for claim in claims {
            let summary = required(claim, "text")?;
            let indices = entries(claim, "sources")?;
            if summary.len() > QUERY_LIMIT || indices.is_empty() {
                return Err("结论缺少可核对来源".into());
            }
            text.push_str("- ");
            text.push_str(summary);
            let mut refs = Vec::new();
            for index in indices {
                let index = index
                    .as_u64()
                    .filter(|i| *i > 0 && *i <= sources.len() as u64)
                    .ok_or("模型引用了未获授权的来源")?;
                text.push_str(&format!(" [{index}]"));
                refs.push(&sources[index as usize - 1]["id"]);
            }
            let claim_id = revision_hash(json!([summary, refs]).to_string().as_bytes());
            used_claims.push(json!({"id":claim_id,"text":summary,"sourceRefs":refs}));
            text.push('\n');
        }
        if let Some(unknown) = result["unknown"].as_str()
            && !unknown.trim().is_empty()
            && unknown.len() <= QUERY_LIMIT
        {
            text.push_str(&format!("\n未决事项\n{unknown}\n"));
        }
        data["possibleDisagreements"] = json!(possible_disagreements(
            &result,
            sources,
            &data["disagreements"]
        ));
        data["usedClaims"] = json!(used_claims);
        data["text"] = json!(text);
        let id = required(&data, "id")?;
        self.connection
            .execute(
                "INSERT INTO m2_answers VALUES(?1,?2)",
                params![id, data.to_string()],
            )
            .map_err(db)?;
        Ok(data)
    }

    pub(crate) fn copy_answer(&self, id: &str) -> M2Result<String> {
        let stored: String = self
            .connection
            .query_row("SELECT data FROM m2_answers WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .map_err(db)?;
        let data = parse(&stored)?;
        let mut text = format!(
            "{}\n\n{}\n",
            required(&data, "query")?,
            required(&data, "text")?
        );
        let scope = Self::answer_scope(&data)?;
        if scope.from.is_some() || scope.to.is_some() {
            text.push_str(&format!(
                "资料日期（{}）{} 至 {}\n",
                if scope.basis == "event" {
                    "事件／发表日期"
                } else {
                    "记录日期"
                },
                scope.from.as_deref().unwrap_or("不限"),
                scope.to.as_deref().unwrap_or("不限")
            ));
        }
        text.push_str("覆盖范围：本地工作区中获确认的片段\n");
        for (index, source) in entries(&data, "sources")?.iter().enumerate() {
            let now = self.verify_evidence(source)?;
            if now["available"] != true {
                return Err("答案依据缺失或不可用，复制前请重新准备并生成答案".into());
            }
            let updated_note = if now["changed"] == true {
                "（原文件已有更新，以下为引用时快照）"
            } else {
                ""
            };
            text.push_str(&format!(
                "[{}] {}，{}–{} 行，{}{}\n{}\n",
                index + 1,
                required(&now, "path")?,
                now["startLine"],
                now["endLine"],
                now["eventDate"]
                    .as_str()
                    .or_else(|| now["date"].as_str())
                    .unwrap_or("日期未知"),
                updated_note,
                required(&now, "excerpt")?
            ));
        }
        let related = self.answer_disagreements(&data)?;
        for item in related.as_array().ok_or("分歧关系无法核对，不能复制结论")? {
            let relation = match required(item, "relation")? {
                "contradiction" => "观点冲突",
                "conditions" => "条件不同",
                "historical" => "历史变化",
                _ => return Err("分歧关系无法核对，不能复制结论".into()),
            };
            text.push_str(&format!("\n{relation}　{}\n", required(item, "question")?));
            for (index, member) in entries(item, "members")?.iter().enumerate() {
                let source = &member["source"];
                let context = if source["outsideRange"] == true {
                    "（原检索范围外的关联依据）"
                } else {
                    ""
                };
                let detail = if source["available"] != true {
                    "依据缺失或不可读取".to_owned()
                } else if source["changed"] == true {
                    format!(
                        "{}（原文件已有更新，此为引用时快照）",
                        required(member, "text")?
                    )
                } else {
                    required(member, "text")?.to_owned()
                };
                text.push_str(&format!(
                    "- 第 {} 方{}　{}　{}\n",
                    index + 1,
                    context,
                    required(source, "path")?,
                    detail
                ));
            }
            if let Some(judgment) = item["judgment"].as_str().filter(|text| !text.is_empty()) {
                text.push_str(&format!(
                    "当前判断　{judgment}\n理由　{}\n",
                    required(item, "reason")?
                ));
            }
        }
        Ok(text)
    }

    fn random_m2_id(&self) -> M2Result<String> {
        self.connection
            .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))
            .map_err(db)
    }
}

fn possible_disagreements(result: &Value, sources: &[Value], known: &Value) -> Vec<Value> {
    let Some(candidates) = result["possibleDisagreements"].as_array() else {
        return Vec::new();
    };
    candidates
        .iter()
        .take(MAX_CLAIMS)
        .filter_map(|candidate| {
            let question = required(candidate, "question").ok()?;
            let relation = required(candidate, "relation").ok()?;
            if question.len() > QUERY_LIMIT
                || !matches!(relation, "contradiction" | "conditions" | "historical")
            {
                return None;
            }
            let [first, second] = entries(candidate, "sources").ok()?.as_slice() else {
                return None;
            };
            let source = |number: &Value| {
                sources.get(usize::try_from(number.as_u64()?).ok()?.checked_sub(1)?)
            };
            let (a, b) = (source(first)?, source(second)?);
            if overlaps(a, b) {
                return None;
            }
            if known.as_array()?.iter().any(|item| {
                let members = match entries(item, "members") {
                    Ok(members) => members,
                    Err(_) => return false,
                };
                members.iter().any(|member| overlaps(a, &member["source"]))
                    && members.iter().any(|member| overlaps(b, &member["source"]))
            }) {
                return None;
            }
            Some(json!({"question":question,"relation":relation,"members":[
            {"text":a["excerpt"],"source":a}, {"text":b["excerpt"],"source":b}]}))
        })
        .collect()
}

const ZH_STOPWORDS: &[&str] = &[
    "什么",
    "怎么",
    "怎样",
    "如何",
    "哪些",
    "哪个",
    "是否",
    "可以",
    "能否",
    "应该",
    "需要",
    "关于",
    "以及",
    "或者",
    "还有",
    "目前",
    "现在",
    "我们",
    "你们",
    "他们",
    "这个",
    "那个",
    "这些",
    "那些",
    "一个",
    "一下",
    "一些",
    "已经",
    "还是",
    "不是",
    "没有",
    "因为",
    "所以",
    "如果",
    "但是",
    "不过",
    "通过",
    "进行",
    "时候",
    "方面",
    "问题",
    "情况",
    "相关",
    "为什么",
];

const EN_STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "are", "was", "were", "what", "how",
    "why", "when", "where", "which", "can", "should", "about", "into",
];

fn search_terms(query: &str, keyword: &str) -> Vec<String> {
    if !keyword.is_empty() {
        return vec![keyword.to_owned()];
    }
    let mut terms = Vec::new();
    let mut fallback = Vec::new();
    let push_unique = |target: &mut Vec<String>, term: String| {
        if !target.contains(&term) {
            target.push(term);
        }
    };
    for word in query.split(|character: char| !character.is_alphanumeric()) {
        if word.is_ascii() && word.len() > 1 {
            let lower = word.to_ascii_lowercase();
            if !EN_STOPWORDS.contains(&lower.as_str()) {
                push_unique(&mut terms, word.to_owned());
            } else {
                push_unique(&mut fallback, word.to_owned());
            }
        } else {
            let letters: Vec<char> = word.chars().collect();
            for triple in letters.windows(3) {
                let term: String = triple.iter().collect();
                if !ZH_STOPWORDS.contains(&term.as_str()) {
                    push_unique(&mut terms, term);
                }
            }
            for pair in letters.windows(2) {
                let term: String = pair.iter().collect();
                if ZH_STOPWORDS.contains(&term.as_str()) {
                    push_unique(&mut fallback, term);
                } else {
                    push_unique(&mut terms, term);
                }
            }
        }
        if terms.len() >= 32 {
            break;
        }
    }
    terms.truncate(32);
    if terms.is_empty() {
        fallback.truncate(32);
        terms = fallback;
    }
    if terms.is_empty() {
        terms.push(query.to_owned());
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::TestDir;
    use std::fs;

    #[test]
    fn one_sided_query_expands_disagreement_and_refuses_stale_copy() {
        let dir = TestDir::new();
        let a = "---\nevent_date: 2026-08-01\n---\n# 当前规则\n\n先看纸鸢规则，再采用。";
        let b = "---\nevent_date: 2025-04-01\n---\n# 旧规则\n\n允许直接改页。";
        fs::write(dir.0.join("a.md"), a).unwrap();
        fs::write(dir.0.join("b.md"), b).unwrap();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".state").unwrap();
        let model = storage
            .set_model_config(
                "http://127.0.0.1/v1/chat/completions".into(),
                "fixture".into(),
                true,
            )
            .unwrap();
        storage
            .search_reader()
            .unwrap()
            .rebuild_search_index()
            .unwrap();
        let disagreement = storage.create_disagreement("纸鸢如何维护", "historical", vec![
            json!({"path":"a.md","revision":revision_hash(a.as_bytes()),"excerpt":"先看纸鸢规则，再采用。"}),
            json!({"path":"b.md","revision":revision_hash(b.as_bytes()),"excerpt":"允许直接改页。"}),
        ]).unwrap();
        let anchored = storage
            .prepare_answer(
                "这条主张有哪些依据？",
                "no-match",
                DateScope::new(None, None, "event".into()).unwrap(),
                "answer",
                disagreement["members"][0]["claimId"].as_str(),
            )
            .unwrap();
        assert!(anchored["initial"].as_array().unwrap().is_empty());
        assert_eq!(anchored["sources"].as_array().unwrap().len(), 2);
        let prepared = storage
            .prepare_answer(
                "怎样维护 Wiki？",
                "纸鸢规则",
                DateScope::new(Some("2026-07-01".into()), None, "event".into()).unwrap(),
                "answer",
                None,
            )
            .unwrap();
        assert_eq!(prepared["initial"].as_array().unwrap().len(), 1);
        assert_eq!(prepared["sources"].as_array().unwrap().len(), 3); // Candidate and two narrower cited passages.
        assert_eq!(prepared["disagreements"][0]["id"], disagreement["id"]);
        assert_eq!(
            prepared["disagreements"][0]["members"][1]["source"]["outsideRange"],
            true
        );
        let id = required(&prepared, "id").unwrap();
        let data = storage.start_answer(id, &model).unwrap();
        assert!(storage.start_answer(id, &model).is_err());
        storage
            .finish_answer(
                data,
                &model,
                json!({"claims":[{"text":"当前需要先看规则","sources":[1]}]}),
            )
            .unwrap();
        let copied = storage.copy_answer(id).unwrap();
        assert!(copied.contains("允许直接改页"));
        assert!(copied.contains("原检索范围外"));
        fs::rename(dir.0.join("a.md"), dir.0.join("current.md")).unwrap();
        storage
            .search_reader()
            .unwrap()
            .rebuild_search_index()
            .unwrap();
        let moved_initial = storage
            .prepare_answer(
                "怎样维护 Wiki？",
                "纸鸢规则",
                DateScope::new(Some("2026-07-01".into()), None, "event".into()).unwrap(),
                "answer",
                None,
            )
            .unwrap();
        assert_eq!(moved_initial["initial"][0]["path"], "current.md");
        assert_eq!(moved_initial["disagreements"][0]["id"], disagreement["id"]);
        fs::rename(dir.0.join("b.md"), dir.0.join("moved.md")).unwrap();
        fs::write(dir.0.join("copied.md"), b).unwrap();
        storage
            .search_reader()
            .unwrap()
            .rebuild_search_index()
            .unwrap();
        let relations = storage.list_disagreements().unwrap();
        assert_eq!(relations[0]["members"][1]["source"]["path"], "moved.md");
        fs::remove_file(dir.0.join("moved.md")).unwrap();
        let relations = storage.list_disagreements().unwrap();
        assert_eq!(relations[0]["members"][1]["source"]["available"], false);
        assert_eq!(relations[0]["members"][1]["text"], "");
        assert!(storage.copy_answer(id).is_err());
        storage
            .connection
            .execute(
                "DELETE FROM disagreement_members WHERE disagreement_id = ?1 AND member_order = 1",
                [disagreement["id"].as_str().unwrap()],
            )
            .unwrap();
        assert!(
            storage
                .list_disagreements()
                .unwrap_err()
                .contains("无法核对")
        );
        assert!(
            storage
                .prepare_answer(
                    "任意问题",
                    "当前规则",
                    DateScope::new(None, None, "recorded".into()).unwrap(),
                    "answer",
                    None,
                )
                .unwrap_err()
                .contains("无法核对")
        );
    }

    #[test]
    fn edited_source_keeps_user_judgment_and_allows_copy_with_notice() {
        let dir = TestDir::new();
        let a = "# 当前方案\n采用方案 A。";
        let b = "# 备选方案\n采用方案 B。";
        fs::write(dir.0.join("a.md"), a).unwrap();
        fs::write(dir.0.join("b.md"), b).unwrap();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".state").unwrap();
        let model = storage
            .set_model_config(
                "http://127.0.0.1/v1/chat/completions".into(),
                "fixture".into(),
                true,
            )
            .unwrap();
        storage
            .search_reader()
            .unwrap()
            .rebuild_search_index()
            .unwrap();
        let disagreement = storage
            .create_disagreement(
                "采用哪个方案",
                "conditions",
                vec![
                    json!({"path":"a.md","revision":revision_hash(a.as_bytes()),"excerpt":"采用方案 A。"}),
                    json!({"path":"b.md","revision":revision_hash(b.as_bytes()),"excerpt":"采用方案 B。"}),
                ],
            )
            .unwrap();
        storage
            .review_disagreement(
                disagreement["id"].as_str().unwrap(),
                "decided",
                "先采用方案 A",
                "当前阶段更稳妥",
            )
            .unwrap();
        let prepared = storage
            .prepare_answer(
                "采用哪个方案？",
                "方案 A",
                DateScope::new(None, None, "recorded".into()).unwrap(),
                "answer",
                None,
            )
            .unwrap();
        let id = required(&prepared, "id").unwrap();
        let data = storage.start_answer(id, &model).unwrap();
        storage
            .finish_answer(
                data,
                &model,
                json!({"claims":[{"text":"当前选择方案 A","sources":[1]}]}),
            )
            .unwrap();

        fs::write(dir.0.join("a.md"), format!("{a}\n补充说明。")).unwrap();
        let listed = storage.list_disagreements().unwrap();
        assert_eq!(listed[0]["judgment"], "先采用方案 A");
        assert_eq!(listed[0]["reason"], "当前阶段更稳妥");
        assert_eq!(listed[0]["sourcesChanged"], true);
        let copied = storage.copy_answer(id).unwrap();
        assert!(copied.contains("原文件已有更新"));
        assert!(copied.contains("先采用方案 A"));
    }

    #[test]
    fn migrates_v3_workspace_with_legacy_disagreements_and_fts5_losslessly() {
        let dir = TestDir::new();
        let a = "# 架构决策 A\n采用本地 SQLite WAL 与增量索引。";
        let b = "# 架构决策 B\n保持单连接直写并全量扫描。";
        fs::write(dir.0.join("a.md"), a).unwrap();
        fs::write(dir.0.join("b.md"), b).unwrap();
        let mut storage = Storage::open(&dir.0, Path::new("."), ".state").unwrap();
        storage.save_draft("draft", 1, "待保存草稿内容").unwrap();
        storage
            .save_record("req-1", "draft", 1, "待保存草稿内容")
            .unwrap();
        storage.save_draft("draft", 2, "最新未提交草稿").unwrap();
        storage
            .set_model_config(
                "http://127.0.0.1/v1/chat/completions".into(),
                "fixture".into(),
                true,
            )
            .unwrap();
        storage
            .search_reader()
            .unwrap()
            .rebuild_search_index()
            .unwrap();
        let created = storage
            .create_disagreement(
                "索引存储选型",
                "contradiction",
                vec![
                    json!({"path":"a.md","revision":revision_hash(a.as_bytes()),"excerpt":"采用本地 SQLite WAL 与增量索引。"}),
                    json!({"path":"b.md","revision":revision_hash(b.as_bytes()),"excerpt":"保持单连接直写并全量扫描。"}),
                ],
            )
            .unwrap();
        let reviewed = storage
            .review_disagreement(
                created["id"].as_str().unwrap(),
                "decided",
                "选用 WAL + 增量索引",
                "并发读写不阻塞",
            )
            .unwrap();
        let legacy_json = reviewed.to_string();
        let legacy_id = reviewed["id"].as_str().unwrap().to_owned();
        let claim_id = reviewed["members"][0]["claimId"]
            .as_str()
            .unwrap()
            .to_owned();

        // Downgrade schema in-place to simulate an authentic v3 database.
        storage
            .connection
            .execute_batch(
                "CREATE TABLE legacy_search_backup AS
                     SELECT relative_path, captured_at, revision, body FROM search_documents;
                 DROP TRIGGER search_documents_ai;
                 DROP TRIGGER search_documents_ad;
                 DROP TRIGGER search_documents_au;
                 DROP TABLE search_documents_fts;
                 DROP TABLE search_documents;
                 CREATE TABLE search_documents (
                     relative_path TEXT PRIMARY KEY, captured_at TEXT, revision TEXT NOT NULL, body TEXT NOT NULL
                 );
                 INSERT INTO search_documents SELECT * FROM legacy_search_backup;
                 DROP TABLE legacy_search_backup;
                 DROP TABLE disagreement_history;
                 DROP TABLE disagreement_members;
                 DROP TABLE disagreements;
                 CREATE TABLE m2_disagreements (id TEXT PRIMARY KEY, data TEXT NOT NULL);
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        storage
            .connection
            .execute(
                "INSERT INTO m2_disagreements (id, data) VALUES (?1, ?2)",
                params![&legacy_id, &legacy_json],
            )
            .unwrap();
        drop(storage);

        // Reopen triggers atomic v3 -> v4 migration.
        let mut migrated = Storage::open(&dir.0, Path::new("."), ".state").unwrap();
        let version: i64 = migrated
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4);
        let legacy_table_exists: bool = migrated
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='m2_disagreements')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!legacy_table_exists);
        assert_eq!(
            migrated.load_draft("draft").unwrap().unwrap().body,
            "最新未提交草稿"
        );
        assert_eq!(migrated.list_recent_captures().unwrap().len(), 1);

        // FTS5 was rebuilt during migration; 3+ char search hits immediately without manual rebuild.
        let hits = migrated
            .search_reader()
            .unwrap()
            .search_records("增量索引", None, None, 0)
            .unwrap()
            .0;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "a.md");

        // Disagreement, judgment, reason, history, and claimId survived intact.
        let relations = migrated.list_disagreements().unwrap();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0]["id"], legacy_id);
        assert_eq!(relations[0]["status"], "decided");
        assert_eq!(relations[0]["judgment"], "选用 WAL + 增量索引");
        assert_eq!(relations[0]["reason"], "并发读写不阻塞");
        assert_eq!(relations[0]["history"].as_array().unwrap().len(), 1);
        assert_eq!(relations[0]["members"][0]["claimId"], claim_id);

        // Indexed claim_id lookup uses idx_disagreement_members_claim.
        let plan: String = migrated
            .connection
            .query_row(
                "EXPLAIN QUERY PLAN SELECT disagreement_id FROM disagreement_members WHERE claim_id = ?1",
                [&claim_id],
                |row| row.get(3),
            )
            .unwrap();
        assert!(
            plan.contains("idx_disagreement_members_claim"),
            "unexpected query plan: {plan}"
        );
        let prepared = migrated
            .prepare_answer(
                "索引如何选型？",
                "",
                DateScope::new(None, None, "recorded".into()).unwrap(),
                "answer",
                Some(&claim_id),
            )
            .unwrap();
        assert_eq!(prepared["disagreements"][0]["id"], legacy_id);
    }
}
