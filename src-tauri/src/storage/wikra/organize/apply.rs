use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use super::edit_ops::{Action, PreparedOp, render, source_units};
use super::*;
use crate::storage::wikra::documents::{Catalog, LinkResolution, file_stem, meta_path, type_label};
use crate::storage::wikra::markdown::{insert_block, line_ending};

const INDEX: &str = "wiki/index.md";
const LOG: &str = "wiki/log.md";

const CHANGESET_FIELDS: &str =
    "SELECT id, created_at, trigger, model_alias, status, summary, note, parent_id, dropped_claims
     FROM wikra_changesets";

fn map_changeset(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChangesetItemDto> {
    Ok(ChangesetItemDto {
        id: row.get(0)?,
        time: row.get(1)?,
        trigger: row.get(2)?,
        model_alias: row.get(3)?,
        status: row.get(4)?,
        summary: row.get(5)?,
        note: row.get(6)?,
        parent_id: row.get(7)?,
        dropped_claims: row.get::<_, u32>(8)? as usize,
        page_paths: Vec::new(),
        pages: Vec::new(),
        ops: Vec::new(),
    })
}

struct SavedPage {
    page: ChangesetPageDto,
    before: Option<String>,
    after_hash: String,
}

fn saved_pages(conn: &Connection, id: &str) -> Result<Vec<SavedPage>, StorageError> {
    let mut query = conn
        .prepare(
            "SELECT path, title, state, before_snapshot, after_hash
         FROM wikra_changeset_pages WHERE changeset_id = ?1 ORDER BY path",
        )
        .map_err(db)?;
    query
        .query_map([id], |row| {
            Ok(SavedPage {
                page: ChangesetPageDto {
                    path: row.get(0)?,
                    title: row.get(1)?,
                    state: row.get(2)?,
                },
                before: row.get(3)?,
                after_hash: row.get(4)?,
            })
        })
        .map_err(db)?
        .collect::<Result<_, _>>()
        .map_err(db)
}

struct SavedOp {
    dto: ChangesetOpDto,
    payload: String,
    base_hash: Option<String>,
}

fn saved_ops(conn: &Connection, id: &str) -> Result<Vec<SavedOp>, StorageError> {
    let mut query = conn.prepare(
        "SELECT o.seq, o.op, o.label, o.page_path, o.page_title,
             CASE WHEN o.outcome = 'applied' AND (p.state = 'reverted' OR c.status = 'reverted')
                  THEN 'reverted' ELSE o.outcome END,
             o.reason, o.conflict_id, o.child_changeset_id, o.payload_json, o.base_hash
         FROM wikra_changeset_ops o
         LEFT JOIN wikra_changeset_pages p ON p.changeset_id = o.changeset_id AND p.path = o.page_path
         LEFT JOIN wikra_changesets c ON c.id = o.child_changeset_id
         WHERE o.changeset_id = ?1 ORDER BY o.seq",
    ).map_err(db)?;
    query
        .query_map([id], |row| {
            Ok(SavedOp {
                dto: ChangesetOpDto {
                    seq: row.get(0)?,
                    op: row.get(1)?,
                    label: row.get(2)?,
                    target_path: row.get(3)?,
                    target_title: row.get(4)?,
                    outcome: row.get(5)?,
                    reason: row.get(6)?,
                    conflict_id: row.get(7)?,
                    child_changeset_id: row.get(8)?,
                },
                payload: row.get(9)?,
                base_hash: row.get(10)?,
            })
        })
        .map_err(db)?
        .collect::<Result<_, _>>()
        .map_err(db)
}

fn decorate(
    conn: &Connection,
    mut changeset: ChangesetItemDto,
) -> Result<ChangesetItemDto, StorageError> {
    let mut query = conn.prepare("SELECT path, title, state FROM wikra_changeset_pages WHERE changeset_id = ?1 ORDER BY path").map_err(db)?;
    changeset.pages = query
        .query_map([&changeset.id], |row| {
            Ok(ChangesetPageDto {
                path: row.get(0)?,
                title: row.get(1)?,
                state: row.get(2)?,
            })
        })
        .map_err(db)?
        .collect::<Result<_, _>>()
        .map_err(db)?;
    changeset.ops = saved_ops(conn, &changeset.id)?
        .into_iter()
        .map(|op| op.dto)
        .collect();
    changeset.page_paths = changeset
        .pages
        .iter()
        .map(|page| page.path.clone())
        .chain(
            changeset
                .ops
                .iter()
                .filter(|op| !op.target_path.is_empty())
                .map(|op| op.target_path.clone()),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(changeset)
}

fn load_changeset(conn: &Connection, id: &str) -> Result<ChangesetItemDto, StorageError> {
    let item = conn
        .query_row(
            &format!("{CHANGESET_FIELDS} WHERE id = ?1"),
            [id],
            map_changeset,
        )
        .optional()
        .map_err(db)?
        .ok_or_else(|| invalid(STAGE, "找不到这次整理记录"))?;
    decorate(conn, item)
}

pub(super) fn list_changesets(conn: &Connection) -> Result<Vec<ChangesetItemDto>, StorageError> {
    let mut query = conn
        .prepare(&format!("{CHANGESET_FIELDS} ORDER BY rowid DESC"))
        .map_err(db)?;
    query
        .query_map([], map_changeset)
        .map_err(db)?
        .map(|row| decorate(conn, row.map_err(db)?))
        .collect()
}

struct Header<'a> {
    id: &'a str,
    time: &'a str,
    trigger: &'a str,
    model: &'a str,
    summary: &'a str,
    dropped_claims: usize,
    parent: Option<&'a str>,
    inbox: Option<&'a str>,
    positions: &'a [i64],
}

fn save_header(
    conn: &Connection,
    header: &Header<'_>,
    ops: &[PreparedOp],
) -> Result<(), StorageError> {
    let main = ops
        .iter()
        .find(|op| op.outcome != "dropped")
        .or_else(|| ops.first());
    let note = if header.dropped_claims > 0 {
        format!("已跳过 {} 条不符原文的结论", header.dropped_claims)
    } else {
        String::new()
    };
    let status = if ops.iter().any(|op| op.outcome == "pending_review") {
        "partial"
    } else {
        "applied"
    };
    conn.execute(
        "INSERT INTO wikra_changesets (
          id, created_at, trigger, model_alias, status, summary, target_path, target_title, op_label,
          before_snapshot, after_hash, inbox_id, paragraphs_json, parent_id, note, dropped_claims
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, '', ?10, ?11, ?12, ?13, ?14)",
        params![header.id, header.time, header.trigger, header.model, status, header.summary,
            main.map_or("", |op| &op.path), main.map_or("", |op| &op.title),
            main.map_or("", |op| &op.label), header.inbox, json!(header.positions).to_string(),
            header.parent, note, header.dropped_claims as i64],
    ).map_err(db)?;
    Ok(())
}

fn save_ops(
    conn: &Connection,
    id: &str,
    ops: &[PreparedOp],
    catalog: &Catalog,
) -> Result<(), StorageError> {
    let mut conflicts = HashMap::new();
    for (index, op) in ops.iter().enumerate() {
        let conflict_id = if op.conflict && op.outcome == "pending_review" {
            if !conflicts.contains_key(&op.path) {
                let current = catalog
                    .paths
                    .get(&op.path.to_lowercase())
                    .map(|&i| {
                        tidy(
                            &catalog.docs[i].body[catalog.docs[i].parsed.content_start..],
                            240,
                        )
                    })
                    .filter(|excerpt| !excerpt.is_empty())
                    .unwrap_or_else(|| "尚无可比对的页面内容".to_owned());
                let incoming = match &op.action {
                    Some(Action::Append { section }) => section.claims.first(),
                    Some(Action::Create { sections, .. } | Action::AppendExisting { sections }) => {
                        sections.first().and_then(|section| section.claims.first())
                    }
                    Some(Action::Replace { claims, .. }) => claims.first(),
                    _ => None,
                }
                .map_or_else(|| op.reason.clone(), |claim| tidy(&claim.text, 240));
                let conflict_id: String = conn.query_row(
                    "INSERT INTO conflicts (id, topic, current_label, current_path, current_text,
                       incoming_label, incoming_text, status, created_at)
                     VALUES (lower(hex(randomblob(16))), ?1, '当前知识页', ?2, ?3, '新材料', ?4,
                             'open', strftime('%Y-%m-%dT%H:%M:%fZ','now')) RETURNING id",
                    params![op.title, op.path, current, incoming], |row| row.get(0),
                ).map_err(db)?;
                conflicts.insert(op.path.clone(), conflict_id);
            }
            conflicts.get(&op.path)
        } else {
            None
        };
        let payload = if op.outcome == "dropped" {
            "{}".to_owned()
        } else {
            json!(op.action).to_string()
        };
        conn.execute(
            "INSERT INTO wikra_changeset_ops (
                changeset_id, seq, op, page_path, page_title, label, payload_json, base_hash, outcome, reason, conflict_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![id, (index + 1) as i64, op.op, op.path, op.title, op.label, payload, op.base_hash, op.outcome, op.reason, conflict_id]
        ).map_err(db)?;
    }
    Ok(())
}

fn save_page(
    conn: &Connection,
    id: &str,
    edit: &MarkdownEdit,
    title: &str,
    after: &str,
) -> Result<(), StorageError> {
    conn.execute(
        "INSERT INTO wikra_changeset_pages (changeset_id, path, title, before_snapshot, after_hash, state)
         VALUES (?1, ?2, ?3, ?4, ?5, 'applied')",
        params![id, edit.key, title, edit.before, revision_hash(after.as_bytes())],
    ).map_err(db)?;
    Ok(())
}

fn in_write_scope(workspace: &WorkspaceStorage, path: &str) -> bool {
    in_directory(path, "wiki")
        && !in_directory(path, &workspace.raw_dir)
        && !in_directory(path, &workspace.capture_dir)
}

fn can_edit(conn: &Connection, workspace: &WorkspaceStorage, path: &str) -> bool {
    in_write_scope(workspace, path)
        && !meta_path(path)
        && !is_capture(conn, path)
        && ensure_editable(conn, path, STAGE).is_ok()
}

fn read_page(files: &WorkspaceFiles, path: &str) -> Result<Option<String>, StorageError> {
    match files.markdown_file(path) {
        Ok((key, file)) if key.eq_ignore_ascii_case(path) => fs::read_to_string(file)
            .map(Some)
            .map_err(|err| source_error(STAGE, err)),
        Ok(_) => Err(conflict(STAGE, "目标页位置已变更")),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(source_error(STAGE, err)),
    }
}

fn page_hash(body: Option<&str>) -> Option<String> {
    body.map(|body| revision_hash(body.as_bytes()))
}

fn ordered_action((index, action): (usize, &Action)) -> (usize, std::cmp::Reverse<usize>, usize) {
    let (order, reverse) = match action {
        Action::Create { .. } => (0, 0),
        Action::FixLink { span, .. } => (1, span.start),
        Action::Replace { heading, .. } => (2, *heading),
        _ => (3, 0),
    };
    (order, std::cmp::Reverse(reverse), index)
}

fn linked(catalog: &Catalog, from: &str, body: &str, to: &str, target: &str) -> bool {
    Markdown::parse(body).links.iter().any(|link| {
        link.target.eq_ignore_ascii_case(target)
            || catalog
                .resolve(from, &link.target)
                .is_some_and(|id| catalog.docs[id].path == to)
    })
}

fn duplicate_heading(body: &str, action: &Action) -> bool {
    let parsed = Markdown::parse(body);
    let repeated = |name: &str| {
        parsed
            .headings
            .iter()
            .filter(|h| h.level == 2 && h.title == name)
            .take(2)
            .count()
            == 2
    };
    match action {
        Action::Append { section } => repeated(&section.heading),
        Action::AppendExisting { sections } => {
            sections.iter().any(|section| repeated(&section.heading))
        }
        Action::Link { .. } => repeated("相关链接"),
        _ => false,
    }
}

fn new_title_taken(catalog: &Catalog, name: &str, target_path: &str) -> bool {
    catalog.named(name).iter().any(|&id| {
        let doc = &catalog.docs[id];
        doc.path != target_path
            && (doc.knowledge_page() || file_stem(&doc.path).to_lowercase() == name.to_lowercase())
    })
}

fn groups(ops: &[PreparedOp]) -> BTreeMap<String, Vec<usize>> {
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for (index, op) in ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.outcome != "dropped")
    {
        groups.entry(op.path.clone()).or_default().push(index);
    }
    for indexes in groups.values_mut() {
        indexes.sort_by_key(|&index| {
            ordered_action((
                index,
                ops[index].action.as_ref().expect("prepared operation"),
            ))
        });
    }
    groups
}

fn created_titles(ops: &[PreparedOp], outcome: Option<&str>) -> HashSet<String> {
    ops.iter()
        .filter(|op| op.outcome != "dropped" && outcome.is_none_or(|outcome| op.outcome == outcome))
        .filter(|op| matches!(op.action, Some(Action::Create { .. })))
        .map(|op| op.title.to_lowercase())
        .collect()
}

fn stage_ops(
    conn: &Connection,
    workspace: &WorkspaceStorage,
    catalog: &Catalog,
    live: &Catalog,
    id: &str,
    time: &str,
    ops: &mut [PreparedOp],
) {
    let proposed = created_titles(ops, None);
    let mut alias_owners = HashMap::new();
    for op in ops.iter().filter(|op| op.outcome != "dropped") {
        if let Some(Action::Alias { value }) = &op.action {
            alias_owners
                .entry(value.to_lowercase())
                .or_insert_with(HashSet::new)
                .insert(op.path.clone());
        }
    }
    for (path, indexes) in groups(ops) {
        if !can_edit(conn, workspace, &path) {
            for index in indexes {
                ops[index].drop("目标页超出可自动整理的范围");
            }
            continue;
        }
        let mut body = catalog
            .paths
            .get(&path.to_lowercase())
            .map(|&id| catalog.docs[id].body.clone());
        let current = read_page(&workspace.files, &path);
        if current
            .as_ref()
            .is_err_and(|err| err.code == "VERSION_CONFLICT")
        {
            for index in indexes {
                ops[index].drop("目标页位置已变更");
            }
            continue;
        }
        let changed = current.as_ref().map_or(true, |current| {
            page_hash(current.as_deref()) != ops[indexes[0]].base_hash
        });
        for index in indexes {
            let op = &mut ops[index];
            let action = op.action.as_ref().expect("prepared operation");
            if let Action::Alias { value } = action
                && (alias_owners[&value.to_lowercase()].len() > 1
                    || live
                        .named(value)
                        .iter()
                        .any(|&id| live.docs[id].path != path))
            {
                op.drop("本批多个页面申请了相同别名");
                continue;
            }
            let title_in_use =
                matches!(action, Action::Create { .. }) && new_title_taken(live, &op.title, &path);
            if let Action::Link {
                path: to, target, ..
            } = action
                && body
                    .as_deref()
                    .is_some_and(|body| linked(catalog, &path, body, to, target))
            {
                continue;
            }
            let duplicate = body
                .as_deref()
                .is_some_and(|body| duplicate_heading(body, action));
            match render(
                action,
                body.as_deref(),
                &op.title,
                time,
                id,
                index + 1,
                true,
                |target| {
                    catalog.resolve(&path, target).is_some()
                        || proposed.contains(&target.to_lowercase())
                },
            ) {
                Ok(after) => {
                    body = Some(after);
                    if title_in_use {
                        op.review("有另一篇同名页，请合并已有页面");
                    }
                    if duplicate {
                        op.review("同名章节有多个，请审阅单独的补充章节");
                    }
                    if changed {
                        op.review("页面刚刚有新变动，本次未写入");
                    }
                }
                Err(reason) => op.drop(reason),
            }
        }
    }
    let created: HashSet<_> = ops
        .iter()
        .filter(|op| op.outcome != "dropped" && matches!(op.action, Some(Action::Create { .. })))
        .map(|op| op.path.clone())
        .collect();
    for op in ops.iter_mut().filter(|op| op.outcome != "dropped") {
        if let Some(Action::Link { path: to, .. }) = &op.action
            && !created.contains(to)
            && !live.paths.contains_key(&to.to_lowercase())
        {
            op.drop("关联的新页没有通过核验");
        }
    }
    let pages = groups(ops);
    if pages.len() > 15 {
        for op in ops.iter_mut() {
            op.review("本批变化超过 15 篇，需逐页确认");
        }
    }
    let mut review: HashSet<_> = ops
        .iter()
        .filter(|op| op.outcome == "pending_review")
        .map(|op| op.path.clone())
        .collect();
    loop {
        let mut added = false;
        for op in ops.iter().filter(|op| op.outcome != "dropped") {
            if let Some(Action::Link { path, .. }) = &op.action
                && created.contains(path)
                && review.contains(path)
            {
                added |= review.insert(op.path.clone());
            }
        }
        if !added {
            break;
        }
    }
    for op in ops.iter_mut().filter(|op| review.contains(&op.path)) {
        op.review("同页有待审改动，或链接目标尚未写入");
    }
}

fn prepare_writes(
    conn: &Connection,
    workspace: &WorkspaceStorage,
    catalog: &Catalog,
    header: &Header<'_>,
    ops: &[PreparedOp],
    separate: bool,
) -> Result<MarkdownBatch, StorageError> {
    let mut batch = MarkdownBatch::default();
    let new = created_titles(ops, Some("applied"));
    for (path, indexes) in groups(ops) {
        let indexes: Vec<_> = indexes
            .into_iter()
            .filter(|&i| ops[i].outcome == "applied")
            .collect();
        let Some(&first) = indexes.first() else {
            continue;
        };
        let edit = MarkdownEdit::load(
            &workspace.files,
            &path,
            ops[first].base_hash.is_none(),
            STAGE,
        )?;
        if !edit.key.eq_ignore_ascii_case(&path)
            || !can_edit(conn, workspace, &edit.key)
            || page_hash(edit.before.as_deref()) != ops[first].base_hash
        {
            return Err(conflict(STAGE, "Markdown 已在外部被修改，请刷新后重试"));
        }
        let mut body = edit.before.clone();
        for index in indexes {
            let action = ops[index].action.as_ref().expect("prepared operation");
            if let Action::Link {
                path: to, target, ..
            } = action
                && body
                    .as_deref()
                    .is_some_and(|body| linked(catalog, &path, body, to, target))
            {
                continue;
            }
            body = Some(
                render(
                    action,
                    body.as_deref(),
                    &ops[index].title,
                    header.time,
                    header.id,
                    index + 1,
                    separate,
                    |target| {
                        catalog.resolve(&path, target).is_some()
                            || new.contains(&target.to_lowercase())
                    },
                )
                .map_err(|reason| conflict(STAGE, reason))?,
            );
        }
        if let Some(body) = body.filter(|body| edit.before.as_ref() != Some(body)) {
            save_page(conn, header.id, &edit, &ops[first].title, &body)?;
            batch.push(edit, Some(body));
        }
    }
    Ok(batch)
}

fn finished_summary(id: &str, ops: &[PreparedOp], remaining: usize) -> CompileSummaryDto {
    let count = |outcome| ops.iter().filter(|op| op.outcome == outcome).count();
    CompileSummaryDto {
        changeset_id: id.to_owned(),
        applied: count("applied"),
        pending: count("pending_review"),
        dropped: count("dropped"),
        remaining_paragraphs: remaining,
    }
}

impl Storage {
    pub(crate) fn apply_inbox_compilation(
        &mut self,
        item: &InboxCompilation,
        model_alias: &str,
        output: CompilationOutput,
    ) -> Result<InboxItemDetailDto, StorageError> {
        let (_, summary) = self.apply_compilation(Compilation::Inbox(item), model_alias, output)?;
        let mut detail = self.inbox_item(&item.id)?;
        detail.compile_summary = Some(summary);
        Ok(detail)
    }

    pub(crate) fn apply_buffer_compilation(
        &mut self,
        batch: &BufferBatch,
        model_alias: &str,
        output: CompilationOutput,
    ) -> Result<(ChangesetItemDto, CompileSummaryDto), StorageError> {
        self.apply_compilation(Compilation::Buffer(batch), model_alias, output)
    }

    fn apply_compilation(
        &mut self,
        compilation: Compilation<'_>,
        model_alias: &str,
        output: CompilationOutput,
    ) -> Result<(ChangesetItemDto, CompileSummaryDto), StorageError> {
        let first = self.commit_compilation(compilation, model_alias, &output);
        let (id, time, summary) = match first {
            Err(err) if err.code == "VERSION_CONFLICT" => {
                self.commit_compilation(compilation, model_alias, &output)?
            }
            result => result?,
        };
        self.bookkeep(&time, "compile", &output.summary);
        Ok((load_changeset(&self.connection, &id)?, summary))
    }

    fn commit_compilation(
        &mut self,
        compilation: Compilation<'_>,
        model_alias: &str,
        output: &CompilationOutput,
    ) -> Result<(String, String, CompileSummaryDto), StorageError> {
        let live = Catalog::load(self)?;
        let (id, time) = database_identity(&self.connection, STAGE)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let files = &self.workspace.files;
        match compilation {
            Compilation::Inbox(item) => item.verify(&tx, files, output.context.source_count)?,
            Compilation::Buffer(batch) => batch.verify(&tx, files, output.context.source_count)?,
        }
        let catalog = &output.context.catalog;
        let mut ops = output.ops.clone();
        stage_ops(&tx, &self.workspace, catalog, &live, &id, &time, &mut ops);
        if ops.iter().all(|op| op.outcome == "dropped") && output.covered.is_empty() && !output.noop
        {
            return Err(error(
                "INVALID_INPUT",
                STAGE,
                None,
                true,
                "模型未生成可核验的整理改动，请重试",
                "no valid operation",
            ));
        }
        let mut covered = output.covered.clone();
        for op in ops.iter().filter(|op| op.outcome != "dropped") {
            covered.extend(source_units(op));
        }
        let (trigger, inbox, positions, total) = match compilation {
            Compilation::Inbox(item) => (
                "收件箱整理",
                Some(item.id.as_str()),
                item.positions
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| covered.contains(i))
                    .map(|(_, &p)| p)
                    .collect::<Vec<_>>(),
                item.positions.len(),
            ),
            Compilation::Buffer(batch) => ("随手记合批整理", None, Vec::new(), batch.sources.len()),
        };
        let header = Header {
            id: &id,
            time: &time,
            trigger,
            model: model_alias,
            summary: &output.summary,
            dropped_claims: output.dropped_claims,
            parent: None,
            inbox,
            positions: &positions,
        };
        save_header(&tx, &header, &ops)?;
        save_ops(&tx, &id, &ops, catalog)?;
        let batch = prepare_writes(&tx, &self.workspace, &live, &header, &ops, false)?;
        match compilation {
            Compilation::Inbox(item) if !positions.is_empty() => {
                set_compiled(&tx, &item.id, &positions, true)?;
                tx.execute("UPDATE inbox_items SET state = 'compiled' WHERE id = ?1 AND state <> 'archived'", [&item.id]).map_err(db)?;
            }
            Compilation::Inbox(_) => (),
            Compilation::Buffer(source) => source.finish(&tx, &covered)?,
        }
        batch.commit(tx, STAGE)?;
        let summary = finished_summary(&id, &ops, total.saturating_sub(covered.len()));
        Ok((id, time, summary))
    }

    pub(crate) fn get_changeset_detail(
        &self,
        id: &str,
    ) -> Result<ChangesetDetailDto, StorageError> {
        let changeset = load_changeset(&self.connection, id)?;
        let saved = saved_pages(&self.connection, id)?;
        let ops = saved_ops(&self.connection, id)?;
        let catalog = Catalog::load(self)?;
        let mut pages = Vec::new();
        for path in &changeset.page_paths {
            let saved = saved.iter().find(|saved| &saved.page.path == path);
            let current = read_page(&self.workspace.files, path);
            let readable = current.is_ok();
            let current = current.ok().flatten();
            let current_hash = page_hash(current.as_deref());
            let matching: Vec<_> = ops
                .iter()
                .filter(|op| &op.dto.target_path == path)
                .collect();
            let pending: Vec<_> = matching
                .iter()
                .copied()
                .filter(|op| op.dto.outcome == "pending_review")
                .collect();
            let has_pending = !pending.is_empty();
            let pending_reasons: BTreeSet<_> =
                pending.iter().map(|op| op.dto.reason.clone()).collect();
            let (before, base_hash, state) = if let Some(saved) = saved {
                let expected = if saved.page.state == "reverted" {
                    page_hash(saved.before.as_deref())
                } else {
                    Some(saved.after_hash.clone())
                };
                (saved.before.clone(), expected, saved.page.state.clone())
            } else if let Some(first) = pending.first() {
                let unchanged = readable && current_hash == first.base_hash;
                (
                    current.clone().filter(|_| unchanged),
                    first.base_hash.clone(),
                    "pending_review".into(),
                )
            } else {
                (
                    None,
                    current_hash.clone(),
                    matching
                        .first()
                        .map_or("discarded", |op| &op.dto.outcome)
                        .into(),
                )
            };
            let externally_modified = !readable || current_hash != base_hash;
            let title = saved
                .map(|page| page.page.title.as_str())
                .or_else(|| matching.first().map(|op| op.dto.target_title.as_str()))
                .unwrap_or(path);
            let replay: Vec<_> = if has_pending {
                pending
            } else {
                matching
                    .iter()
                    .copied()
                    .filter(|op| {
                        matches!(op.dto.outcome.as_str(), "applied" | "reverted")
                            && op.dto.child_changeset_id.is_none()
                    })
                    .collect()
            };
            let after = if !has_pending
                && saved.is_some_and(|page| page.page.state == "applied")
                && !externally_modified
            {
                current.clone()
            } else {
                replay_page(
                    &catalog,
                    path,
                    title,
                    if has_pending {
                        current.as_deref()
                    } else {
                        before.as_deref()
                    },
                    &changeset,
                    &replay,
                )
            };
            let has_protected_section = current
                .as_ref()
                .or(before.as_ref())
                .is_some_and(|body| Markdown::parse(body).protected());
            pages.push(ChangesetPageDiffDto {
                page: ChangesetPageDto {
                    path: path.clone(),
                    title: title.to_owned(),
                    state,
                },
                has_pending,
                pending_reasons: pending_reasons.into_iter().collect(),
                has_protected_section,
                before,
                after,
                current,
                externally_modified,
            });
        }
        Ok(ChangesetDetailDto { changeset, pages })
    }

    pub(crate) fn decide_review(
        &mut self,
        id: &str,
        path: &str,
        decision: ReviewDecision,
    ) -> Result<ChangesetItemDto, StorageError> {
        let catalog = Catalog::load(self)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let parent = load_changeset(&tx, id)?;
        let pending: Vec<_> = saved_ops(&tx, id)?
            .into_iter()
            .filter(|op| op.dto.target_path == path && op.dto.outcome == "pending_review")
            .collect();
        if pending.is_empty() {
            return Err(invalid(STAGE, "这篇页面当前没有待审改动"));
        }
        let mut book = None;
        let batch = match decision {
            ReviewDecision::Discard => {
                tx.execute("UPDATE wikra_changeset_ops SET outcome = 'discarded' WHERE changeset_id = ?1 AND page_path = ?2 AND outcome = 'pending_review'", params![id, path]).map_err(db)?;
                MarkdownBatch::default()
            }
            ReviewDecision::Apply => {
                let first = &pending[0];
                let current = read_page(&self.workspace.files, path)
                    .map_err(|_| conflict(STAGE, "页面无法读取，请检查磁盘版本"))?;
                if page_hash(current.as_deref()) != first.base_hash
                    || pending.iter().any(|op| op.base_hash != first.base_hash)
                {
                    return Err(conflict(STAGE, "页面已有外部改动，请还原基线或手工合并"));
                }
                let mut sources = HashMap::new();
                let ops = pending
                    .iter()
                    .map(|op| {
                        let action: Action = serde_json::from_str(&op.payload)
                            .map_err(|_| invalid(STAGE, "待审记录无法读取"))?;
                        match &action {
                            Action::Link { path: target, .. }
                                if !catalog.paths.contains_key(&target.to_lowercase()) =>
                            {
                                return Err(conflict(STAGE, "请先采纳链接指向的新页"));
                            }
                            Action::Alias { value }
                                if catalog
                                    .named(value)
                                    .iter()
                                    .any(|&i| catalog.docs[i].path != path) =>
                            {
                                return Err(conflict(STAGE, "别名已由其他页面使用，请手工合并"));
                            }
                            Action::Create { .. }
                                if new_title_taken(&catalog, &op.dto.target_title, path) =>
                            {
                                return Err(conflict(STAGE, "已有同名知识页，请手工合并"));
                            }
                            _ => (),
                        }
                        for claim in action.claims() {
                            if !sources.contains_key(&claim.locator) {
                                let source =
                                    read_source_bytes(&self.workspace.files, &claim.locator)
                                        .ok()
                                        .and_then(|bytes| String::from_utf8(bytes).ok())
                                        .unwrap_or_default();
                                sources.insert(claim.locator.clone(), source);
                            }
                            if !sources[&claim.locator].contains(&claim.quote) {
                                return Err(conflict(STAGE, "待审改动的短引已无法在原文核验"));
                            }
                        }
                        Ok(PreparedOp::new(
                            path.to_owned(),
                            op.dto.target_title.clone(),
                            op.base_hash.clone(),
                            action,
                        ))
                    })
                    .collect::<Result<Vec<_>, StorageError>>()?;
                let (child, time) = database_identity(&tx, STAGE)?;
                let inbox: Option<String> = tx
                    .query_row(
                        "SELECT inbox_id FROM wikra_changesets WHERE id = ?1",
                        [id],
                        |row| row.get(0),
                    )
                    .map_err(db)?;
                let header = Header {
                    id: &child,
                    time: &time,
                    trigger: "采纳待审改动",
                    model: &parent.model_alias,
                    summary: &parent.summary,
                    dropped_claims: 0,
                    parent: Some(id),
                    inbox: inbox.as_deref(),
                    positions: &[],
                };
                save_header(&tx, &header, &ops)?;
                save_ops(&tx, &child, &ops, &catalog)?;
                let batch = prepare_writes(&tx, &self.workspace, &catalog, &header, &ops, true)?;
                tx.execute(
                    "UPDATE wikra_changeset_ops SET outcome = 'applied', child_changeset_id = ?3
                    WHERE changeset_id = ?1 AND page_path = ?2 AND outcome = 'pending_review'",
                    params![id, path, child],
                )
                .map_err(db)?;
                book = Some((time, parent.summary));
                batch
            }
        };
        close_conflicts(&tx, id)?;
        tx.execute("UPDATE wikra_changesets SET status = CASE WHEN EXISTS(
                        SELECT 1 FROM wikra_changeset_ops WHERE changeset_id = ?1 AND outcome = 'pending_review'
                    ) THEN 'partial' ELSE 'applied' END WHERE id = ?1", [id]).map_err(db)?;
        batch.commit(tx, STAGE)?;
        if let Some((time, summary)) = book {
            self.bookkeep(&time, "review", &summary);
        }
        load_changeset(&self.connection, id)
    }

    pub(crate) fn revert_changeset(
        &mut self,
        id: &str,
    ) -> Result<RevertChangesetResultDto, StorageError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let changeset = load_changeset(&tx, id)?;
        let mut restored = Vec::new();
        let mut kept_modified = Vec::new();
        if changeset.status == "reverted" {
            return Ok(RevertChangesetResultDto {
                changeset,
                restored,
                kept_modified,
            });
        }
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM wikra_changesets WHERE parent_id = ?1 AND status <> 'reverted')",
            [id], |row| row.get::<_, bool>(0)).map_err(db)? {
            return Err(conflict(STAGE, "请先撤销这次整理下已采纳的子批次"));
        }
        let mut batch = MarkdownBatch::default();
        for saved in saved_pages(&tx, id)? {
            let path = &saved.page.path;
            if saved.page.state == "reverted" {
                continue;
            }
            let edit = can_edit(&tx, &self.workspace, path)
                .then(|| MarkdownEdit::load(&self.workspace.files, path, false, STAGE).ok())
                .flatten()
                .filter(|edit| {
                    edit.key.eq_ignore_ascii_case(path)
                        && edit.before.as_ref().is_some_and(|before| {
                            revision_hash(before.as_bytes()) == saved.after_hash
                        })
                });
            let state = if let Some(edit) = edit {
                restored.push(path.clone());
                batch.push(edit, saved.before);
                "reverted"
            } else {
                kept_modified.push(path.clone());
                "kept_modified"
            };
            tx.execute(
                "UPDATE wikra_changeset_pages SET state = ?3 WHERE changeset_id = ?1 AND path = ?2",
                params![id, path, state],
            )
            .map_err(db)?;
        }
        tx.execute("UPDATE wikra_changeset_ops SET outcome = 'discarded' WHERE changeset_id = ?1 AND outcome = 'pending_review'", [id]).map_err(db)?;
        close_conflicts(&tx, id)?;
        let all = kept_modified.is_empty();
        tx.execute(
            "UPDATE wikra_changesets SET status = ?2 WHERE id = ?1",
            params![
                id,
                if all {
                    "reverted"
                } else {
                    "partially_reverted"
                }
            ],
        )
        .map_err(db)?;
        if all {
            let (inbox, positions): (Option<String>, String) = tx
                .query_row(
                    "SELECT inbox_id, paragraphs_json FROM wikra_changesets WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(db)?;
            if let Some(inbox) = inbox {
                let positions: Vec<i64> = serde_json::from_str(&positions)
                    .map_err(|_| invalid(STAGE, "整理记录的段落索引无法读取"))?;
                set_compiled(&tx, &inbox, &positions, false)?;
                tx.execute(
                    "UPDATE inbox_items SET state = CASE WHEN EXISTS(
                    SELECT 1 FROM inbox_paragraphs WHERE item_id = ?1 AND compiled <> 0
                 ) THEN 'compiled' ELSE 'reading' END WHERE id = ?1 AND state <> 'archived'",
                    [&inbox],
                )
                .map_err(db)?;
            }
        }
        let (_, time) = database_identity(&tx, STAGE)?;
        batch.commit(tx, STAGE)?;
        self.bookkeep(&time, "revert", &changeset.summary);
        Ok(RevertChangesetResultDto {
            changeset: load_changeset(&self.connection, id)?,
            restored,
            kept_modified,
        })
    }

    pub(in crate::storage::wikra) fn commit_lint_edits(
        &mut self,
        catalog: &Catalog,
        ops: Vec<PreparedOp>,
    ) -> Result<ChangesetItemDto, StorageError> {
        let (id, time) = database_identity(&self.connection, STAGE)?;
        let header = Header {
            id: &id,
            time: &time,
            trigger: "结构体检",
            model: "",
            summary: "修复知识页结构问题",
            dropped_claims: 0,
            parent: None,
            inbox: None,
            positions: &[],
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        save_header(&tx, &header, &ops)?;
        save_ops(&tx, &id, &ops, catalog)?;
        prepare_writes(&tx, &self.workspace, catalog, &header, &ops, false)?.commit(tx, STAGE)?;
        self.bookkeep(&time, "lint", header.summary);
        load_changeset(&self.connection, &id)
    }
}

fn replay_page(
    catalog: &Catalog,
    path: &str,
    title: &str,
    before: Option<&str>,
    changeset: &ChangesetItemDto,
    ops: &[&SavedOp],
) -> Option<String> {
    if ops.is_empty() {
        return None;
    }
    let mut actions: Vec<_> = ops
        .iter()
        .map(|op| {
            serde_json::from_str::<Action>(&op.payload)
                .ok()
                .map(|action| (op.dto.seq as usize, action))
        })
        .collect::<Option<_>>()?;
    actions.sort_by_key(|(seq, action)| ordered_action((*seq, action)));
    let mut body = before.map(str::to_owned);
    for (seq, action) in actions {
        if let Action::Link {
            path: to, target, ..
        } = &action
            && body
                .as_deref()
                .is_some_and(|body| linked(catalog, path, body, to, target))
        {
            continue;
        }
        body = Some(
            render(
                &action,
                body.as_deref(),
                title,
                &changeset.time,
                &changeset.id,
                seq,
                true,
                |target| {
                    catalog.resolve(path, target).is_some() || target.eq_ignore_ascii_case(title)
                },
            )
            .ok()?,
        );
    }
    body
}

fn close_conflicts(conn: &Connection, id: &str) -> Result<(), StorageError> {
    conn.execute(
        "UPDATE conflicts SET status = 'keep_both', resolution_note = '关联待审改动已处理',
            resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE status = 'open' AND id IN (SELECT conflict_id FROM wikra_changeset_ops WHERE changeset_id = ?1)
            AND NOT EXISTS (SELECT 1 FROM wikra_changeset_ops WHERE conflict_id = conflicts.id AND outcome = 'pending_review')",
        [id],
    ).map_err(db)?;
    Ok(())
}

pub(crate) fn index_block(
    catalog: &Catalog,
    compact: bool,
    include: impl Fn(&crate::storage::wikra::documents::Document) -> bool,
) -> String {
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    let mut count = 0;
    for doc in catalog
        .docs
        .iter()
        .filter(|doc| doc.knowledge_page() && include(doc))
    {
        groups
            .entry(type_label(&doc.doc_type))
            .or_default()
            .push(doc);
        count += 1;
    }
    let mut text = "<!-- wikra-index:start -->".to_owned();
    if count == 0 {
        text.push_str("\n\n知识库暂无知识页。");
    }
    for (kind, mut pages) in groups {
        text.push_str(&format!("\n\n## {kind}\n"));
        pages.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.path.cmp(&b.path))
        });
        let max = if compact || count > 200 {
            30
        } else {
            pages.len()
        };
        for doc in pages.iter().take(max) {
            let stem = file_stem(&doc.path);
            let target = if matches!(catalog.resolve_link("", stem), LinkResolution::Unique(i) if catalog.docs[i].path == doc.path)
            {
                stem
            } else {
                &doc.path[..doc.path.len() - 3]
            };
            text.push_str(&format!(
                "\n- [[{}|{}]]",
                target,
                doc.title.replace(['[', ']', '|', '\r', '\n'], " ")
            ));
        }
        if pages.len() > max {
            text.push_str(&format!("\n- 其余 {} 页", pages.len() - max));
        }
    }
    text.push_str("\n\n<!-- wikra-index:end -->");
    text
}

pub(in crate::storage::wikra) fn managed_block(
    body: &str,
    block: &str,
    start: &str,
    end: &str,
) -> Option<String> {
    let lb = line_ending(body);
    let raw = block.replace('\n', lb);
    let parsed = Markdown::parse(body);
    let mut starts = parsed
        .managed_markers
        .iter()
        .filter(|range| &body[(*range).clone()] == start);
    let mut ends = parsed
        .managed_markers
        .iter()
        .filter(|range| &body[(*range).clone()] == end);
    match (starts.next(), ends.next()) {
        (None, None) => Some(insert_block(body, body.len(), block)),
        (Some(at), Some(until))
            if at.start < until.start && starts.next().is_none() && ends.next().is_none() =>
        {
            let range = at.start..until.end;
            if parsed.intersects_protection(&range) {
                return None;
            }
            Some(format!(
                "{}{raw}{}",
                &body[..range.start],
                &body[range.end..]
            ))
        }
        _ => None,
    }
}

pub(in crate::storage::wikra) fn index_after(
    catalog: &Catalog,
    before: Option<&str>,
) -> Option<String> {
    let original = before.unwrap_or("# 知识库索引\n");
    let parsed = Markdown::parse(original);
    if parsed.managed_false() || parsed.blocked() {
        return None;
    }
    managed_block(
        original,
        &index_block(catalog, false, |_| true),
        "<!-- wikra-index:start -->",
        "<!-- wikra-index:end -->",
    )
}

impl Storage {
    pub(in crate::storage::wikra) fn rebuild_wikra_index(
        &self,
        catalog: &Catalog,
    ) -> Result<bool, StorageError> {
        if !in_write_scope(&self.workspace, INDEX)
            || is_capture(&self.connection, INDEX)
            || ensure_editable(&self.connection, INDEX, STAGE).is_err()
        {
            return Ok(false);
        }
        let before = read_page(&self.workspace.files, INDEX)?;
        let Some(after) =
            index_after(catalog, before.as_deref()).filter(|after| before.as_ref() != Some(after))
        else {
            return Ok(false);
        };
        let edit = MarkdownEdit::load(&self.workspace.files, INDEX, true, STAGE)?;
        if edit.key != INDEX || edit.before != before {
            return Err(conflict(STAGE, "知识库目录已在外部被修改"));
        }
        write_edit(&edit.path, before.as_deref(), Some(&after))
            .map_err(|err| source_error(STAGE, err))?;
        Ok(true)
    }

    fn bookkeep(&self, time: &str, action: &str, summary: &str) {
        if Catalog::load(self)
            .and_then(|catalog| self.rebuild_wikra_index(&catalog))
            .is_err()
        {
            eprintln!("Wikra 目录更新失败");
        }
        if !in_write_scope(&self.workspace, LOG)
            || is_capture(&self.connection, LOG)
            || ensure_editable(&self.connection, LOG, STAGE).is_err()
        {
            return;
        }
        let append = || -> io::Result<()> {
            let (key, path) = self.workspace.files.create_markdown_target(LOG)?;
            if key != LOG {
                return Err(io::Error::other("log path changed"));
            }
            let before = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
                Err(err) => return Err(err),
            };
            if Markdown::parse(&before).blocked() {
                return Err(io::Error::other("log structure is unclosed"));
            }
            let lb = line_ending(&before);
            let prefix = if before.is_empty() {
                format!("# 整理日志{lb}{lb}")
            } else if !before.ends_with('\n') {
                lb.to_owned()
            } else {
                String::new()
            };
            let text = format!("{prefix}- {time} [{action}] {}{lb}", tidy(summary, 320));
            fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(path)?
                .write_all(text.as_bytes())?;
            Ok(())
        };
        if append().is_err() {
            eprintln!("Wikra 整理日志追加失败");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::TestDir;
    use crate::storage::wikra::tests::standard_vault;

    #[test]
    fn markdown_batch_rolls_back_files_and_index_in_reverse_on_a_late_conflict() {
        let dir = TestDir::new();
        let mut storage = standard_vault(&dir);
        let a = "wiki/A.md";
        let b = "wiki/B.md";
        storage.create_markdown_document(a, "alpha").unwrap();
        storage.create_markdown_document(b, "beta").unwrap();
        let files = storage.workspace_files();
        let mut batch = MarkdownBatch::default();
        batch.push(
            MarkdownEdit::load(&files, a, false, STAGE).unwrap(),
            Some("changed alpha".into()),
        );
        batch.push(
            MarkdownEdit::load(&files, b, false, STAGE).unwrap(),
            Some("changed beta".into()),
        );
        fs::write(dir.0.join(b), "external beta").unwrap();
        let tx = storage
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            batch.commit(tx, STAGE).unwrap_err().code,
            "VERSION_CONFLICT"
        );
        assert_eq!(fs::read_to_string(dir.0.join(a)).unwrap(), "alpha");
        assert_eq!(fs::read_to_string(dir.0.join(b)).unwrap(), "external beta");
        let indexed: Vec<String> = storage
            .connection
            .prepare("SELECT body FROM search_documents ORDER BY relative_path")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(indexed, ["alpha", "beta"]);
    }

    #[test]
    fn sixteen_content_changes_move_the_whole_batch_to_review() {
        let dir = TestDir::new();
        let mut storage = standard_vault(&dir);
        for n in 1..=16 {
            storage
                .create_markdown_document(&format!("wiki/Page{n}.md"), &format!("# Page{n}\n"))
                .unwrap();
        }
        let catalog = Catalog::load(&storage).unwrap();
        let mut ops: Vec<_> = catalog
            .docs
            .iter()
            .filter(|doc| doc.knowledge_page())
            .map(|doc| {
                PreparedOp::new(
                    doc.path.clone(),
                    doc.title.clone(),
                    page_hash(Some(&doc.body)),
                    Action::Alias {
                        value: format!("Reviewed {}", doc.title),
                    },
                )
            })
            .collect();
        stage_ops(
            &storage.connection,
            &storage.workspace,
            &catalog,
            &catalog,
            "0000000000000000",
            "2026-11-27",
            &mut ops,
        );
        assert_eq!(ops.len(), 16);
        assert!(
            ops.iter()
                .all(|op| op.outcome == "pending_review" && op.reason.contains("15"))
        );
    }
}
