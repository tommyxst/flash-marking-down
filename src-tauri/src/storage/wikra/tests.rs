use serde_json::json;

use crate::workspace::{LEGACY_STATE_DIR, STANDARD_CAPTURE_DIR, STATE_DIR};

use super::super::tests::TestDir;
use super::inbox::{CompileMode, prepare_clip};
use super::organize::edit_ops::fixture_output;
use super::organize::{
    CaptureMode, CompilationOutput, ModelAccess, Source, generate_teachback, local_teachback,
};
use super::*;

pub(super) fn standard_vault(dir: &TestDir) -> Storage {
    let storage = Storage::open(&dir.0, Path::new(STANDARD_CAPTURE_DIR), STATE_DIR).unwrap();
    storage
        .workspace_files()
        .initialize_standard_layout()
        .unwrap();
    storage
}

fn fixture_response(
    title: &str,
    target: Option<&str>,
    points: &[(usize, &str)],
) -> serde_json::Value {
    let points: Vec<_> = points.iter()
        .map(|(source, quote)| json!({ "source": format!("s{source}"), "quote": quote, "text": format!("要点：{quote}") }))
        .collect();
    let units: Vec<_> = points.iter().map(|point| &point["source"]).collect();
    let (target, op) = if let Some(target) = target {
        (
            json!(target),
            json!({"op":"append_section","page":"p1","heading":title,"claims":points}),
        )
    } else {
        (
            json!({"new":{"type":"entity","title":title}}),
            json!({"op":"create_page","page":"n1","lead":"","sections":[{"heading":"要点","claims":points}]}),
        )
    };
    json!({"routes":[{"units":units,"target":target,"relation":"extends","why":""}],"summary":"整理摘要","ops":[op]})
}

fn compiled_output(
    storage: &mut Storage,
    sources: &[Source],
    title: &str,
    target: Option<&str>,
    points: &[(usize, &str)],
) -> CompilationOutput {
    let context = storage.prepare_compilation(sources).unwrap();
    fixture_output(context, sources, fixture_response(title, target, points))
}

#[test]
fn initializes_standard_vault_without_replacing_existing_schema() {
    let dir = TestDir::new();
    let schema = dir.0.join("SCHEMA.md");
    fs::write(&schema, "# 自定义管理规则").unwrap();
    let storage = standard_vault(&dir);
    storage
        .workspace_files()
        .initialize_standard_layout()
        .unwrap();
    assert_eq!(fs::read_to_string(schema).unwrap(), "# 自定义管理规则");
    assert_eq!(storage.capture_dir(), "raw/notes");
    for subdir in ["decisions", "concepts", "syntheses", "entities"] {
        assert!(dir.0.join("wiki").join(subdir).is_dir());
    }
    assert!(dir.0.join("raw/clips").is_dir());
    assert!(dir.0.join(".wikra/state.sqlite").is_file());

    let fresh = TestDir::new();
    standard_vault(&fresh);
    assert!(
        fs::read_to_string(fresh.0.join("SCHEMA.md"))
            .unwrap()
            .contains("Wikra")
    );
}

#[test]
fn opens_existing_obsidian_vault_without_creating_default_raw_dir() {
    let dir = TestDir::new();
    fs::create_dir(dir.0.join(".obsidian")).unwrap();
    fs::write(dir.0.join("Existing.md"), "# Obsidian 知识页").unwrap();
    let storage = Storage::open(&dir.0, Path::new("daily"), STATE_DIR).unwrap();
    assert_eq!(storage.raw_dir(), "raw");
    assert_eq!(storage.list_library_tree().unwrap().raw_dir, "raw");
    assert!(!dir.0.join("raw").exists());

    storage
        .connection
        .execute(
            "INSERT INTO workspace_settings VALUES ('raw_dir', '.wikra/sources')",
            [],
        )
        .unwrap();
    drop(storage);
    let reopened = Storage::open(&dir.0, Path::new("daily"), STATE_DIR).unwrap();
    assert_eq!(reopened.raw_dir(), "raw");
    assert!(!dir.0.join("raw").exists());
}

#[test]
fn custom_raw_dir_persists_and_uses_existing_obsidian_files_in_place() {
    let dir = TestDir::new();
    let raw_dir = "00-Raw/sources";
    let source = "00-Raw/sources/evidence.md";
    let source_body = "# 采样记录\n\n极光观测原始数据";
    let page = "Topics/summary.md";
    let old_raw = "raw/legacy.md";
    fs::create_dir_all(dir.0.join(raw_dir)).unwrap();
    fs::create_dir_all(dir.0.join("Topics").join(raw_dir)).unwrap();
    fs::write(dir.0.join(source), source_body).unwrap();
    fs::write(dir.0.join("Topics").join(source), "# 同名知识页").unwrap();
    fs::write(
        dir.0.join(page),
        format!("# 极光观测\n\n参考原文[^s1]\n\n[^s1]: [[{source}|数据]] —— “采样记录”"),
    )
    .unwrap();

    let mut storage = Storage::open(&dir.0, Path::new("daily"), STATE_DIR).unwrap();
    let old_clip = storage
        .create_inbox_item(prepare_clip("极光观测旧收藏".into(), Some("旧收藏".into())).unwrap())
        .unwrap()
        .summary
        .raw_path;
    fs::write(dir.0.join(old_raw), "# 旧有原始文件").unwrap();

    let (updated_raw, capture_dir) = storage
        .set_workspace_dirs(Path::new(r"00-Raw\sources"), None)
        .unwrap();
    assert_eq!(updated_raw, raw_dir);
    assert_eq!(capture_dir, "daily");
    let clip = storage
        .create_inbox_item(prepare_clip("极光观测新收藏".into(), Some("新收藏".into())).unwrap())
        .unwrap()
        .summary
        .raw_path;
    assert!(clip.starts_with("00-Raw/sources/clips/"));
    assert_eq!(
        fs::read_to_string(dir.0.join(&clip)).unwrap(),
        "极光观测新收藏"
    );
    drop(storage);

    let mut storage = Storage::open(&dir.0, Path::new("daily"), STATE_DIR).unwrap();
    assert_eq!(storage.raw_dir(), raw_dir);
    let tree = storage.list_library_tree().unwrap();
    assert_eq!(tree.raw_dir, raw_dir);
    assert!(
        tree.raw_files
            .iter()
            .any(|doc| doc.path == source && doc.kind == "note")
    );
    for path in [&old_clip, &clip] {
        assert!(
            tree.raw_files
                .iter()
                .any(|doc| &doc.path == path && doc.kind == "clip")
        );
    }
    assert_eq!(
        storage.read_document_detail(page).unwrap().citations[0].raw_path,
        source
    );
    for path in [source, old_clip.as_str()] {
        assert_eq!(
            storage
                .append_human_understanding(path, "人工理解")
                .unwrap_err()
                .code,
            "INVALID_INPUT",
        );
    }
    assert_eq!(fs::read_to_string(dir.0.join(source)).unwrap(), source_body);
    assert_eq!(
        fs::read_to_string(dir.0.join(old_raw)).unwrap(),
        "# 旧有原始文件"
    );
    assert_eq!(
        fs::read_to_string(dir.0.join(&old_clip)).unwrap(),
        "极光观测旧收藏"
    );
    assert!(!dir.0.join(raw_dir).join("legacy.md").exists());

    let hits = storage.global_search("极光观测", 20).unwrap();
    for (path, kind) in [
        (page, "wiki"),
        (source, "raw"),
        (&old_clip, "raw"),
        (&clip, "raw"),
    ] {
        assert!(
            hits.iter()
                .any(|hit| hit.target_path == path && hit.kind == kind),
            "{path}"
        );
    }
}

#[test]
fn rejects_raw_dirs_outside_document_scope() {
    let dir = TestDir::new();
    let root = dir.0.join("workspace");
    fs::create_dir(&root).unwrap();
    let mut storage = Storage::open(&root, Path::new("daily"), STATE_DIR).unwrap();
    for path in [
        Path::new(""),
        Path::new("."),
        Path::new(".."),
        Path::new("../outside"),
        Path::new("folder/../raw"),
        dir.0.as_path(),
        root.as_path(),
        Path::new(".wikra"),
        Path::new(".flash-marking-down"),
        Path::new("nested/.WiKrA/sources"),
        Path::new("invalid?/sources"),
    ] {
        assert!(storage.set_workspace_dirs(path, None).is_err(), "{path:?}");
    }
    assert_eq!(storage.raw_dir(), "raw");
    assert!(!root.join("raw").exists());
    assert!(!root.join("nested").exists());
    assert!(!dir.0.join("outside").exists());
}

#[test]
fn migrates_v1_v2_v5_and_v6_in_place_and_reopens_legacy_state() {
    for version in [1, 2, 5, 6] {
        let dir = TestDir::new();
        let old_state = dir.0.join(LEGACY_STATE_DIR);
        fs::create_dir(&old_state).unwrap();
        let conn = Connection::open(old_state.join("state.sqlite")).unwrap();
        conn.execute_batch(super::super::SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO workspace_settings VALUES ('id','legacy-id');
             INSERT INTO drafts VALUES ('draft','旧版草稿',3,3,'2026-11-26T08:00:00Z');",
        )
        .unwrap();
        if version == 2 {
            conn.execute_batch("CREATE TABLE search_documents (
                relative_path TEXT PRIMARY KEY, captured_at TEXT, revision TEXT NOT NULL, body TEXT NOT NULL);
                CREATE TABLE search_status (id INTEGER PRIMARY KEY, generation INTEGER NOT NULL, skipped INTEGER NOT NULL);").unwrap();
        }
        if version >= 5 {
            conn.execute_batch(search::SCHEMA).unwrap();
            conn.execute_batch(super::super::NOTES_SCHEMA).unwrap();
            conn.execute_batch(wiki::SCHEMA).unwrap();
            conn.execute_batch(m2_answer::SCHEMA).unwrap();
        }
        conn.pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        conn.pragma_update(None, "user_version", version).unwrap();
        drop(conn);

        for _ in 0..2 {
            let storage = Storage::open(&dir.0, Path::new("old-notes"), STATE_DIR).unwrap();
            assert_eq!(storage.workspace_id(), "legacy-id");
            assert_eq!(
                storage.load_draft("draft").unwrap().unwrap().body,
                "旧版草稿"
            );
            let version: i64 = storage
                .connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(version, 8);
            for name in [
                "inbox_items",
                "inbox_paragraphs",
                "wikra_changesets",
                "capture_buffer_queue",
                "dismissed_suggestions",
                "dismissed_graph_gaps",
            ] {
                assert!(table_exists(&storage.connection, name).unwrap(), "{name}");
            }
            assert!(!dir.0.join(STATE_DIR).exists());
        }
    }
}

#[test]
fn library_and_detail_unify_registered_raw_and_wiki_markdown() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let article = storage
        .create_inbox_item(
            prepare_clip("原始引文\n\n次段".to_owned(), Some("网页原文".into())).unwrap(),
        )
        .unwrap();
    storage
        .create_markdown_document("wiki/entities/Subject.md", "# 实体\n")
        .unwrap();
    let body = format!(
        r#"---
title: "续传: 决策"
type: decision
aliases: ['恢复,别名', "另一个"]
tags:
  - 播放
  - wikra-core
updated: 2026-11-26
fmd_pending_conflicts: [legacy-id]
wikra_pending_conflicts:
  - "待复核"
---
# 运行时标题
## 正常章节

- [ ] 复核结论
  2. [X] 原文已确认

引用[^s1]。关联 [[wiki/entities/Subject|对象]]。

~~~
## 代码示例
- [ ] 忽略
[[不存在]]
~~~

<!-- fmd-protect:start -->
## 我的理解

人工内容。
<!-- fmd-protect:end -->

[^s1]: [[{}#段落|原文标题]] —— “原始引文”
"#,
        article.summary.raw_path
    );
    let decision = "wiki/decisions/Resume.md";
    storage.create_markdown_document(decision, &body).unwrap();
    storage.connection.execute(
        "INSERT INTO notes (id,request_id,source_id,relative_path,captured_at,content_hash,draft_id,draft_revision,body_hash,saved_at,updated_at)
         VALUES ('old','old','old','旧日记.md','2026-11-25','hash','quick-capture',1,'hash','2026-11-25','2026-11-25')",
        [],
    ).unwrap();
    fs::write(dir.0.join("旧日记.md"), "已登记的历史随手记").unwrap();
    fs::create_dir(dir.0.join(LEGACY_STATE_DIR)).unwrap();
    for dir_name in [STATE_DIR, LEGACY_STATE_DIR] {
        fs::write(dir.0.join(dir_name).join("secret.md"), "# internal").unwrap();
        fs::write(dir.0.join(dir_name).join("secret.png"), b"png").unwrap();
    }
    fs::write(dir.0.join("wiki/bad.md"), [0xff]).unwrap();

    let tree = storage.list_library_tree().unwrap();
    assert!(
        tree.raw_files
            .iter()
            .any(|doc| doc.path == "旧日记.md" && doc.kind == "note")
    );
    assert!(
        tree.raw_files
            .iter()
            .any(|doc| doc.path == article.summary.raw_path && doc.linked_page_paths == [decision])
    );
    assert!(tree.wiki_pages.iter().any(|doc| doc.path == decision
        && doc.wikra_core
        && doc.citation_count == 1
        && doc.pending_conflict_count == 2));
    assert!(
        !tree
            .wiki_pages
            .iter()
            .any(|doc| doc.path.contains("secret") || doc.path.contains("bad"))
    );
    let detail = storage.read_document_detail(decision).unwrap();
    assert_eq!(detail.title, "续传: 决策");
    assert_eq!(detail.kind, "wiki");
    assert_eq!(detail.updated_at.as_deref(), Some("2026-11-26"));
    assert_eq!(detail.aliases, ["恢复,别名", "另一个"]);
    assert_eq!(detail.outline, ["正常章节", "我的理解"]);
    assert_eq!(detail.tasks.len(), 2);
    assert!(!detail.tasks[0].done && detail.tasks[1].done);
    assert_eq!(
        body.lines().nth(detail.tasks[1].line_number - 1),
        Some(detail.tasks[1].expected_line.as_str())
    );
    assert!(detail.has_protected_section);
    assert_eq!(detail.citations[0].raw_path, article.summary.raw_path);
    assert_eq!(detail.citations[0].quote, "原始引文");
    let task = &detail.tasks[1];
    assert_eq!(task.id, format!("line:{}", task.line_number));
    assert!(
        storage
            .toggle_task(decision, &task.id, &task.expected_line, false)
            .unwrap()
    );
    let updated = storage.read_document_detail(decision).unwrap();
    assert!(!updated.tasks[1].done);
    assert!(updated.body.contains("2. [ ] 原文已确认"));
    let original = storage
        .read_document_detail(&article.summary.raw_path)
        .unwrap();
    assert_eq!(original.linked_wiki_paths, [decision]);
    assert_eq!(original.backlinks[0].page_path, decision);
    let entity = storage
        .read_document_detail("wiki/entities/Subject.md")
        .unwrap();
    assert_eq!(entity.backlinks[0].page_path, decision);
    let view = serde_json::to_value(detail).unwrap();
    assert!(view["contentHash"].is_string() && view["tasks"][0]["expectedLine"].is_string());
    assert_eq!(view["citations"][0]["refLabel"], "s1");
    for name in [STATE_DIR, LEGACY_STATE_DIR] {
        assert!(
            storage
                .read_document_detail(&format!("{name}/secret.md"))
                .is_err()
        );
        assert!(
            storage
                .workspace_files()
                .read_local_image(Some("wiki/page.md"), &format!("../{name}/secret.png"))
                .is_err()
        );
    }
}

#[test]
fn saves_with_disk_version_conflict_and_updates_fts_without_a_rescan() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let path = "wiki/concepts/Version.md";
    storage
        .create_markdown_document(path, "# 第一版\nformer-keyword")
        .unwrap();
    let stale = storage.read_document_detail(path).unwrap();
    assert_eq!(
        storage
            .create_markdown_document(path, "")
            .err()
            .unwrap()
            .code,
        "VERSION_CONFLICT"
    );
    fs::write(dir.0.join(path), "# 外部改动\nexternally-edited").unwrap();
    assert_eq!(
        storage
            .save_markdown_document(path, &stale.content_hash, "# 不能覆盖")
            .unwrap_err()
            .code,
        "VERSION_CONFLICT",
    );
    let current = storage.read_document_detail(path).unwrap();
    assert!(current.body.contains("externally-edited"));
    storage
        .save_markdown_document(path, &current.content_hash, "# 最新文档\nupdated-keyword")
        .unwrap();
    let hits: i64 = storage.connection.query_row(
        "SELECT count(*) FROM search_documents_fts WHERE search_documents_fts MATCH '\"updated-keyword\"'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(hits, 1);
    assert!(
        storage
            .create_markdown_document("../escape.md", "无")
            .is_err()
    );
    assert!(
        storage
            .create_markdown_document("wiki\\..\\.wikra\\escape.md", "无")
            .is_err()
    );
}

#[test]
fn failed_metadata_commit_restores_the_file_and_fts() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let path = "wiki/Stable.md";
    storage
        .create_markdown_document(path, "original-keyword")
        .unwrap();
    let tx = storage
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    tx.pragma_update(None, "defer_foreign_keys", true).unwrap();
    tx.execute(
        "INSERT INTO inbox_paragraphs (item_id, position, heading, text) VALUES ('missing',0,'','')", [],
    ).unwrap();
    let edit = MarkdownEdit::load(&storage.workspace.files, path, false, "test").unwrap();
    assert_eq!(
        edit.commit(tx, Some("new-keyword"), "test")
            .unwrap_err()
            .code,
        "WRITE_FAILED"
    );
    assert_eq!(
        fs::read_to_string(dir.0.join(path)).unwrap(),
        "original-keyword"
    );
    let indexed: String = storage
        .connection
        .query_row(
            "SELECT body FROM search_documents WHERE relative_path = ?1",
            [path],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(indexed, "original-keyword");
}

#[test]
fn adds_understanding_before_footnotes_or_inside_either_protected_marker() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    for marker in ["wikra", "fmd"] {
        let path = format!("wiki/{marker}.md");
        let body = format!(
            "# 标题\r\n\r\n<!-- {marker}-protect:start -->\r\n原保留句\r\n<!-- {marker}-protect:end -->\r\n\r\n[^s1]: 引文\r\n"
        );
        storage.create_markdown_document(&path, &body).unwrap();
        storage
            .append_human_understanding(&path, "我的新增领悟")
            .unwrap();
        let file = fs::read_to_string(dir.0.join(&path)).unwrap();
        assert!(file.contains("原保留句\r\n\r\n我的新增领悟\r\n"));
        assert!(
            file.find("我的新增领悟").unwrap()
                < file.find(&format!("{marker}-protect:end")).unwrap()
        );
        assert_eq!(file.matches("-protect:start").count(), 1);
    }
    let path = "wiki/no-protect.md";
    storage
        .create_markdown_document(path, "# 标题\n\n[^s1]: 引文\n")
        .unwrap();
    storage
        .append_human_understanding(path, "我的新增领悟")
        .unwrap();
    let body = storage.read_document_detail(path).unwrap().body;
    assert!(body.contains("<!-- wikra-protect:end -->\n\n[^s1]: 引文"));
    let bad = "wiki/malformed.md";
    let before = "# 标题\n<!-- wikra-protect:start -->\n尚未闭合";
    storage.create_markdown_document(bad, before).unwrap();
    assert_eq!(
        storage
            .append_human_understanding(bad, "无")
            .unwrap_err()
            .code,
        "VERSION_CONFLICT"
    );
    assert_eq!(fs::read_to_string(dir.0.join(bad)).unwrap(), before);
}

#[test]
fn saves_stub_receipts_and_only_completes_the_original_once() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let clip = prepare_clip("http://127.0.0.1:19/only-synthetic".to_owned(), None).unwrap();
    let stub = storage.create_inbox_item(clip).unwrap();
    assert_eq!(stub.summary.fetch_status, "stub");
    assert!(stub.paragraphs.is_empty());
    assert!(stub.summary.raw_path.starts_with("raw/clips/"));
    let path = dir.0.join(&stub.summary.raw_path);
    let receipt = fs::read_to_string(&path).unwrap();
    assert!(receipt.contains("/only-synthetic"));
    fs::write(&path, "外部编辑的收据").unwrap();
    assert_eq!(
        storage
            .complete_inbox_stub(&stub.summary.id, "替换")
            .err()
            .unwrap()
            .code,
        "VERSION_CONFLICT"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "外部编辑的收据");
    fs::write(&path, receipt).unwrap();

    let pasted = "## 正文\r\n\r\n第一段\r\n\r\n第二段";
    let full = storage
        .complete_inbox_stub(&stub.summary.id, pasted)
        .unwrap();
    assert_eq!(full.summary.fetch_status, "full");
    assert_eq!(full.paragraphs.len(), 2);
    assert_eq!(fs::read_to_string(path).unwrap(), pasted);
    assert_eq!(storage.list_inbox_items().unwrap().len(), 1);
    let raw = storage
        .read_document_detail(&full.summary.raw_path)
        .unwrap();
    assert_eq!(raw.source_url.as_deref(), full.summary.url.as_deref());
    assert_eq!(
        storage
            .save_markdown_document(&raw.path, &raw.content_hash, "覆写原文")
            .unwrap_err()
            .code,
        "INVALID_INPUT",
    );
    assert_eq!(
        storage
            .update_note(&raw.path, &raw.content_hash, "旧命令也不能覆写")
            .unwrap_err()
            .code,
        "INVALID_INPUT"
    );
    assert!(
        storage
            .complete_inbox_stub(&full.summary.id, "第二次补全")
            .is_err()
    );
    let archived = storage.archive_inbox_item(&full.summary.id).unwrap();
    assert_eq!(archived.summary.state, "archived");
    assert_eq!(fs::read_to_string(dir.0.join(&raw.path)).unwrap(), pasted);
}

#[test]
fn compiles_only_marked_inbox_paragraphs_preserves_protection_and_reverts_safely() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let source = "# 合并规范\n\n第一段可引用\n\n[[第一轮]] 第二段待阅读\r\n";
    let clip = storage
        .create_inbox_item(prepare_clip(source.to_owned(), None).unwrap())
        .unwrap();
    let id = &clip.summary.id;
    assert!(ModelAccess::load(&storage).unwrap().is_none());
    assert_eq!(
        storage
            .prepare_inbox_compile(id, CompileMode::Excerpt)
            .err()
            .unwrap()
            .code,
        "INVALID_INPUT"
    );
    storage
        .toggle_inbox_paragraph(id, &clip.paragraphs[0].id)
        .unwrap();
    let plan = storage
        .prepare_inbox_compile(id, CompileMode::Excerpt)
        .unwrap()
        .unwrap();
    assert_eq!(plan.sources.len(), 1);
    assert_eq!(plan.sources[0].text, "第一段可引用");
    let invalid = compiled_output(
        &mut storage,
        &plan.sources,
        "伪造",
        None,
        &[(1, "不在选段中的引文")],
    );
    assert_eq!(
        storage
            .apply_inbox_compilation(&plan, "fixture-model", invalid)
            .err()
            .unwrap()
            .code,
        "INVALID_INPUT",
    );
    let output = compiled_output(
        &mut storage,
        &plan.sources,
        "第一轮",
        None,
        &[(1, "第一段可引用")],
    );
    let partial = storage
        .apply_inbox_compilation(&plan, "fixture-model", output)
        .unwrap();
    assert!(partial.paragraphs[0].compiled);
    assert!(!partial.paragraphs[1].compiled);
    assert!(
        storage
            .toggle_inbox_paragraph(id, &clip.paragraphs[0].id)
            .is_err()
    );
    let first_change = storage
        .list_organize_overview()
        .unwrap()
        .changesets
        .remove(0);
    let page = &partial.related_page_paths[0];
    storage
        .append_human_understanding(page, "人工整理，不得丢失")
        .unwrap();
    let protected = storage.read_document_detail(page).unwrap().body;

    let plan = storage
        .prepare_inbox_compile(id, CompileMode::Full)
        .unwrap()
        .unwrap();
    assert!(plan.sources.len() == 1 && plan.sources[0].text.contains("第二段待阅读"));
    let output = compiled_output(
        &mut storage,
        &plan.sources,
        "第二轮",
        Some("c1"),
        &[(1, "第二段待阅读")],
    );
    let full = storage
        .apply_inbox_compilation(&plan, "fixture-model", output)
        .unwrap();
    assert!(full.paragraphs.iter().all(|p| p.compiled));
    let page_after = storage.read_document_detail(page).unwrap();
    assert!(page_after.body.contains("人工整理，不得丢失"));
    assert_eq!(page_after.citations.len(), 2);
    assert_eq!(page_after.citations[0].raw_path, clip.summary.raw_path);
    let change = storage
        .list_organize_overview()
        .unwrap()
        .changesets
        .remove(0);
    assert_eq!(change.page_paths, [page.as_str()]);
    assert_eq!(
        storage
            .revert_changeset(&change.id)
            .unwrap()
            .changeset
            .status,
        "reverted"
    );
    assert_eq!(storage.read_document_detail(page).unwrap().body, protected);
    let first_revert = storage.revert_changeset(&first_change.id).unwrap();
    assert_eq!(first_revert.changeset.status, "partially_reverted");
    assert_eq!(first_revert.kept_modified, [page.as_str()]);
    let reverted = storage.inbox_item(id).unwrap();
    assert!(reverted.paragraphs[0].compiled && !reverted.paragraphs[1].compiled);
    assert_eq!(
        fs::read_to_string(dir.0.join(&clip.summary.raw_path)).unwrap(),
        source
    );
}

#[test]
fn capture_queue_is_idempotent_keeps_the_first_deadline_and_reverts_a_new_page() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let record = |s: &mut Storage, id, revision, body, mode| {
        s.save_record_with_buffer(id, "draft", revision, body, Some(mode))
            .unwrap()
    };
    let local = record(&mut storage, "local", 1, "仅存", CaptureMode::Only);
    assert!(!storage.capture_receipt(local).unwrap().queued);
    let a = record(
        &mut storage,
        "queue-a",
        2,
        "缓冲原文一",
        CaptureMode::Queue(60),
    );
    let first_deadline: i64 = storage
        .connection
        .query_row("SELECT due_at FROM capture_buffer_queue", [], |row| {
            row.get(0)
        })
        .unwrap();
    let b = record(
        &mut storage,
        "queue-b",
        3,
        "待移出原文",
        CaptureMode::Queue(300),
    );
    let b_deadline: i64 = storage
        .connection
        .query_row(
            "SELECT due_at FROM capture_buffer_queue WHERE note_id=?1",
            [&b.record_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(first_deadline, b_deadline);
    assert!((1..=60).contains(&storage.buffer_seconds_left().unwrap()));
    assert!(storage.due_buffer_items().unwrap().is_empty());
    storage.cancel_buffer_item(&b.record_id).unwrap();
    let retried = record(
        &mut storage,
        "queue-b",
        3,
        "待移出原文",
        CaptureMode::Queue(60),
    );
    assert_eq!(retried.record_id, b.record_id);
    assert!(!storage.capture_receipt(retried).unwrap().queued);
    assert_eq!(
        storage.get_note_detail(&b.record_id).unwrap().body,
        "待移出原文"
    );

    storage.flush_buffer_now().unwrap();
    assert_eq!(storage.due_buffer_items().unwrap(), [a.record_id.clone()]);
    storage.pause_buffer_items(&[a.record_id.clone()]).unwrap();
    assert!(storage.due_buffer_items().unwrap().is_empty());
    assert_eq!(
        storage.list_organize_overview().unwrap().buffer_queue.len(),
        1
    );
    let c = record(&mut storage, "queue-c", 4, "立即原文二", CaptureMode::Now);
    let ids = storage.due_buffer_items().unwrap();
    assert_eq!(ids, [a.record_id.clone(), c.record_id.clone()]);
    let batch = storage.prepare_buffer_batch(&ids).unwrap();
    let output = compiled_output(
        &mut storage,
        &batch.sources,
        "合批",
        None,
        &[(1, "缓冲原文一"), (2, "立即原文二")],
    );
    let (changeset, _) = storage
        .apply_buffer_compilation(&batch, "fixture-model", output)
        .unwrap();
    assert!(
        storage
            .list_organize_overview()
            .unwrap()
            .buffer_queue
            .is_empty()
    );
    let compiled = storage
        .read_document_detail(&changeset.page_paths[0])
        .unwrap();
    assert_eq!(compiled.citations.len(), 2);
    assert!(
        compiled
            .citations
            .iter()
            .all(|citation| citation.raw_path == a.relative_path)
    );
    let change = storage.revert_changeset(&changeset.id).unwrap();
    assert_eq!(change.changeset.status, "reverted");
    assert!(!dir.0.join(&changeset.page_paths[0]).exists());
    assert!(dir.0.join(&a.relative_path).is_file());
}

#[test]
fn graph_links_citations_gaps_dismissals_and_three_search_sources_are_real() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let raw = "raw/notes/协议.md";
    let a = "wiki/concepts/A.md";
    let hub = "wiki/entities/B.md";
    let other = "wiki/concepts/Other.md";
    storage
        .create_markdown_document(raw, "# 引文原文\n\n光学信号是可检索的原始证据")
        .unwrap();
    storage
        .create_markdown_document(hub, "---\naliases: [B 的别名]\n---\n# B")
        .unwrap();
    storage.create_markdown_document(a, &format!(
        "# 光学信号\n## 采样条件\n\n参见 [[B 的别名]] 和 [[缺失页]]。[^s1]\n\n[^s1]: [协议](<../../{raw}>) —— “光学信号”\n",
    )).unwrap();
    storage
        .create_markdown_document(
            other,
            &format!("# 记录\n\n使用原文[^s1]\n\n[^s1]: [[{raw}|协议]] —— “光学信号”"),
        )
        .unwrap();
    storage
        .create_markdown_document("wiki/C.md", "# C\n\n[[B]]")
        .unwrap();
    storage
        .create_markdown_document("wiki/D.md", "# D\n\n[[wiki/entities/B#标题|B]]")
        .unwrap();
    storage
        .create_markdown_document(
            "wiki/Orphan.md",
            r"# 孤立页

\[[B]] 不建立边。",
        )
        .unwrap();
    storage
        .create_markdown_document("wiki/index.md", "# 导航\n[[B]]")
        .unwrap();
    storage
        .create_markdown_document("wiki/log.md", "# 日志\n[[B]]")
        .unwrap();
    storage
        .create_markdown_document("outside.md", "# 光学信号（其他目录）")
        .unwrap();

    let graph = storage.get_knowledge_graph(false).unwrap();
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.id == hub && node.is_hub && node.degree == 3)
    );
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.id == "wiki/Orphan.md" && node.is_orphan)
    );
    assert!(
        !graph
            .nodes
            .iter()
            .any(|node| node.id == "wiki/index.md" || node.id == "wiki/log.md")
    );
    assert!(
        storage
            .get_knowledge_graph(true)
            .unwrap()
            .nodes
            .iter()
            .any(|node| node.id == "wiki/index.md")
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == a && edge.to == hub && edge.kind == "link")
    );
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.from == a && edge.to == raw && edge.kind == "citation")
    );
    assert!(
        graph
            .gaps
            .iter()
            .any(|gap| gap.from_path == a && gap.to_path == other)
    );
    let missing = graph.gaps.iter().find(|gap| gap.title == "缺失页").unwrap();
    storage.dismiss_graph_gap(&missing.id).unwrap();
    assert!(
        !storage
            .get_knowledge_graph(false)
            .unwrap()
            .gaps
            .iter()
            .any(|gap| gap.id == missing.id)
    );
    let mut overview = storage.list_organize_overview().unwrap();
    let orphan = overview
        .suggestions
        .iter()
        .find(|s| s.page_path == "wiki/Orphan.md")
        .unwrap();
    storage.dismiss_suggestion(&orphan.id).unwrap();
    overview = storage.list_organize_overview().unwrap();
    assert!(
        !overview
            .suggestions
            .iter()
            .any(|s| s.page_path == "wiki/Orphan.md")
    );

    let hits = storage.global_search("光学信号", 20).unwrap();
    for source in ["wiki", "raw", "citation"] {
        assert!(hits.iter().any(|hit| hit.kind == source));
    }
    assert!(!hits.iter().any(|hit| hit.target_path == "outside.md"));
    assert_eq!(storage.global_search("光", 1).unwrap().len(), 1);
    assert!(storage.global_search(" ", 50).unwrap().is_empty());
    let page = storage.read_document_detail(a).unwrap();
    assert!(hits.iter().any(|hit| hit.target_path == a
        && hit.citation_id.as_deref() == Some(page.citations[0].id.as_str())));
    assert!(local_teachback(&page, 1, None).prompt.contains("采样条件"));
    assert!(
        local_teachback(&page, 2, Some("信号"))
            .feedback
            .contains("光学信号")
    );
    assert_eq!(
        local_teachback(&page, 3, Some("先验证采样条件"))
            .suggested_insight
            .as_deref(),
        Some("先验证采样条件")
    );
}

#[test]
fn enabled_model_receives_selected_evidence_and_generates_teachback() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let config = crate::model::ModelSettings::new(&endpoint, "fixture-model", true).unwrap();
    storage.save_service_config("model", &config, "").unwrap();
    let server = std::thread::spawn(move || {
        for stage in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = BufReader::new(stream);
            let mut length = None;
            loop {
                let mut header = String::new();
                assert!(request.read_line(&mut header).unwrap() > 0);
                if header.trim().is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
            let mut bytes = vec![0; length.unwrap()];
            request.read_exact(&mut bytes).unwrap();
            let request_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(request_json["model"], "fixture-model");
            let input: serde_json::Value =
                serde_json::from_str(request_json["messages"][1]["content"].as_str().unwrap())
                    .unwrap();
            let completion = if stage == 0 {
                assert_eq!(input["sources"].as_array().unwrap().len(), 1);
                assert_eq!(input["sources"][0]["text"], "这句获准送入整理");
                fixture_response("选段整理", None, &[(1, "这句获准送入整理")])
            } else {
                assert_eq!(input["step"], 2);
                assert_eq!(input["citations"][0]["quote"], "这句获准送入整理");
                json!({"prompt":"适用边界是什么？","feedback":"补充触发条件。","suggestedInsight":null})
            };
            let response = json!({
                "choices": [{ "finish_reason": "stop", "message": { "content": completion.to_string() } }]
            }).to_string();
            write!(request.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    });
    let body = "# 两段\n\n这句获准送入整理\n\n不发送的另一段";
    let inbox = storage
        .create_inbox_item(prepare_clip(body.to_owned(), None).unwrap())
        .unwrap();
    storage
        .toggle_inbox_paragraph(&inbox.summary.id, &inbox.paragraphs[0].id)
        .unwrap();
    let plan = storage
        .prepare_inbox_compile(&inbox.summary.id, CompileMode::Excerpt)
        .unwrap()
        .unwrap();
    let model = ModelAccess::load(&storage).unwrap().unwrap();
    let context = storage.prepare_compilation(&plan.sources).unwrap();
    let output = model.compile(context, &plan.sources).unwrap();
    let compiled = storage
        .apply_inbox_compilation(&plan, &model.alias, output)
        .unwrap();
    let page = storage
        .read_document_detail(&compiled.related_page_paths[0])
        .unwrap();
    let recalled = generate_teachback(&model, &page, 2, Some("我的试答")).unwrap();
    assert_eq!(recalled.prompt, "适用边界是什么？");
    assert_eq!(recalled.feedback, "补充触发条件。");
    server.join().unwrap();
}

#[test]
fn persists_one_chat_answer_with_its_evidence_and_keeps_protected_text() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let source = "raw/notes/citation.md";
    storage
        .create_markdown_document(source, "# 本地原文\n\n证据句。")
        .unwrap();
    storage
        .connection
        .execute(
            "INSERT INTO conversations VALUES ('conv','如何回查', '2026-11-26', '2026-11-26')",
            [],
        )
        .unwrap();
    let citations = json!([{ "index": 1, "kind": "raw", "path": source, "title": "原文",
        "snippet": "证据句。", "capturedAt": null }])
    .to_string();
    storage
        .connection
        .execute(
            "INSERT INTO chat_messages (id,conversation_id,role,content,citations_json,created_at)
         VALUES ('reply','conv','assistant','结论来自原文[1]。',?1,'2026-11-26')",
            [&citations],
        )
        .unwrap();
    let path = storage.save_answer_to_wiki("conv", "reply", None).unwrap();
    assert!(path.starts_with("wiki/syntheses/"));
    let page = storage.read_document_detail(&path).unwrap();
    assert_eq!(page.citations[0].raw_path, source);
    assert_eq!(page.citations[0].quote, "证据句。");
    storage.save_answer_to_wiki("conv", "reply", None).unwrap();
    assert_eq!(storage.read_document_detail(&path).unwrap().body, page.body);
    let target = "wiki/concepts/Old.md";
    storage
        .create_markdown_document(target, "# 原知识\n")
        .unwrap();
    storage
        .append_human_understanding(target, "手工内容")
        .unwrap();
    storage
        .save_answer_to_wiki("conv", "reply", Some(target))
        .unwrap();
    let page = storage.read_document_detail(target).unwrap();
    assert!(page.has_protected_section && page.body.contains("手工内容"));
    assert!(
        storage
            .save_answer_to_wiki("conv", "reply", Some(source))
            .is_err()
    );
}
