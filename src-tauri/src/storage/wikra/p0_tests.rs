use serde_json::{Value, json};

use crate::dto::wikra::ReviewDecision;
use crate::workspace::{LEGACY_STATE_DIR, STATE_DIR};

use super::super::tests::TestDir;
use super::documents::Catalog;
use super::inbox::{CompileMode, prepare_clip};
use super::markdown::{Markdown, match_quote, upsert_frontmatter_list};
use super::organize::Source;
use super::organize::edit_ops::fixture_output;
use super::organize::route::CompileContext;
use super::tests::standard_vault;
use super::*;

fn wiki_page(storage: &mut Storage, path: &str, body: &str) {
    storage.create_markdown_document(path, body).unwrap();
}

fn clip(storage: &mut Storage, body: &str) -> super::inbox::InboxCompilation {
    let item = storage
        .create_inbox_item(prepare_clip(body.to_owned(), Some("整理样本".into())).unwrap())
        .unwrap();
    storage
        .prepare_inbox_compile(&item.summary.id, CompileMode::Full)
        .unwrap()
        .unwrap()
}

fn candidate(context: &CompileContext, path: &str) -> String {
    let index = context
        .candidates
        .iter()
        .position(|&id| context.catalog.docs[id].path == path)
        .unwrap();
    format!("c{}", index + 1)
}

#[test]
fn migrates_v7_changesets_once_and_retains_the_only_snapshot() {
    let dir = TestDir::new();
    let state = dir.0.join(LEGACY_STATE_DIR);
    fs::create_dir(&state).unwrap();
    let raw = "\u{feff}原文不要改\r\n";
    fs::create_dir(dir.0.join("raw")).unwrap();
    fs::write(dir.0.join("raw/original.md"), raw).unwrap();
    fs::create_dir(dir.0.join("wiki")).unwrap();
    let prior = "# 迁移之前\r\n";
    let after = "# 迁移之后\r\n";
    fs::write(dir.0.join("wiki/legacy.md"), after).unwrap();
    let conn = Connection::open(state.join("state.sqlite")).unwrap();
    for schema in [
        super::super::SCHEMA,
        search::SCHEMA,
        super::super::NOTES_SCHEMA,
        wiki::SCHEMA,
        m2_answer::SCHEMA,
        SCHEMA,
    ] {
        conn.execute_batch(schema).unwrap();
    }
    conn.pragma_update(None, "application_id", APPLICATION_ID)
        .unwrap();
    conn.pragma_update(None, "user_version", 7).unwrap();
    conn.execute("INSERT INTO workspace_settings VALUES ('id','test-v7')", [])
        .unwrap();
    conn.execute(
        "INSERT INTO wikra_changesets(id,created_at,trigger,model_alias,status,summary,target_path,target_title,op_label,before_snapshot,after_hash)
         VALUES ('legacy-1','2026-01-01','迁移','fixture','applied','旧摘要','wiki/legacy.md','旧标题','旧操作',?1,?2),
                ('legacy-2','2026-01-01','迁移','fixture','reverted','旧摘要','wiki/removed.md','旧标题','旧操作',NULL,'removed')",
        params![prior, revision_hash(after.as_bytes())],
    ).unwrap();
    drop(conn);

    let mut store = Storage::open(&dir.0, Path::new("notes"), STATE_DIR).unwrap();
    let version: i64 = store
        .connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 8);
    let parent_snapshot_count: i64 = store
        .connection
        .query_row(
            "SELECT count(*) FROM wikra_changesets WHERE before_snapshot IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(parent_snapshot_count, 0);
    let old = store.get_changeset_detail("legacy-1").unwrap();
    assert_eq!(old.pages[0].before.as_deref(), Some(prior));
    assert_eq!(old.pages[0].after.as_deref(), Some(after));
    assert_eq!(old.changeset.ops[0].op, "append_section");
    let removed = store.get_changeset_detail("legacy-2").unwrap();
    assert_eq!(removed.pages[0].page.state, "reverted");
    assert_eq!(removed.changeset.ops[0].outcome, "discarded");
    assert_eq!(
        store.revert_changeset("legacy-1").unwrap().restored,
        ["wiki/legacy.md"]
    );
    assert_eq!(
        fs::read_to_string(dir.0.join("wiki/legacy.md")).unwrap(),
        prior
    );
    assert_eq!(
        fs::read_to_string(dir.0.join("raw/original.md")).unwrap(),
        raw
    );
    drop(store);
    let store = Storage::open(&dir.0, Path::new("notes"), STATE_DIR).unwrap();
    assert_eq!(store.list_organize_overview().unwrap().changesets.len(), 2);
    assert!(!dir.0.join(STATE_DIR).exists());
}

#[test]
fn markdown_spans_anchors_and_frontmatter_preserve_obsidian_bytes() {
    let before = "\u{feff}---\r\nplugin:\r\n  nested: [x, y]\r\naliases: [\"旧名\", 'A, B'] # 别名注释\r\n\
status: active\r\nsupersedes:\r\n  - Older\r\nsuperseded_by: []\r\nagent_export:\r\n  enabled: true\r\n---\r\n\
# One\r\n## Two\r\n### Three\r\n#### Four\r\n##### Five\r\n###### 六级 标题\r\n段落 ^here-1\r\n\
%%\r\n## 隐藏\r\n[[隐藏]]\r\n%%\r\n```txt\r\n## fenced\r\n[[fake]] ^fake\r\n```\r\n`[[code]] ^fake` [[#六级 标题]]\r\n\
<!-- wikra-protect:start -->\r\nHUMAN 1\r\n<!-- wikra-protect:end -->\r\n\
<!-- fmd-protect:start -->\r\nHUMAN 2\r\n<!-- fmd-protect:end -->\r\n";
    let parsed = Markdown::parse(before);
    assert_eq!(
        parsed
            .headings
            .iter()
            .map(|heading| heading.level)
            .collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6]
    );
    assert!(parsed.has_anchor("六级-标题"));
    assert!(parsed.has_anchor("^here-1"));
    assert!(!parsed.has_anchor("^fake"));
    assert_eq!(parsed.links.len(), 1);
    assert_eq!(&before[parsed.links[0].span.clone()], "[[#六级 标题]]");
    assert_eq!(parsed.protections.len(), 2);
    assert_eq!(parsed.value("status"), Some("active"));
    assert_eq!(parsed.list("supersedes"), ["Older"]);
    assert!(!parsed.blocked());
    let after = upsert_frontmatter_list(before, "aliases", "新增").unwrap();
    assert_eq!(after, before.replace("'A, B']", "'A, B', \"新增\"]"));
    let block = "\u{feff}---\r\naliases: # note\r\n    - 'One' # same\r\n# next\r\nunknown: 7\r\n---\r\n# H\r\n";
    assert_eq!(
        upsert_frontmatter_list(block, "aliases", "Two").unwrap(),
        block.replace("# same\r\n", "# same\r\n    - \"Two\"\r\n")
    );
    let after = parsed
        .append_link_item(before, "- [[Other]]", false)
        .unwrap();
    assert_eq!(
        after
            .trim_end_matches("\r\n")
            .trim_end_matches("- [[Other]]")
            .trim_end_matches("\r\n")
            .trim_end_matches("## 相关链接")
            .trim_end_matches("\r\n"),
        before.trim_end_matches("\r\n")
    );
    assert!(after.lines().all(|line| !line.ends_with('\r')));
    assert_eq!(after.matches('\n').count(), after.matches("\r\n").count());
    assert_eq!(
        match_quote("前一行\r\n关键\u{3000}\t规则\r\n下一行", "关键 规则"),
        Some("关键\u{3000}\t规则")
    );
    assert_eq!(match_quote("跨\r行", "跨 行"), None);
}

#[test]
fn route_combines_links_two_character_names_fts_and_neighbors() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let pages = [
        ("wiki/concepts/网关.md", "# 网关\n\n概念页。"),
        (
            "wiki/concepts/gateway.md",
            "---\naliases: [重试]\n---\n# Gateway\n\n无正文命中。",
        ),
        ("wiki/concepts/Explicit.md", "# Explicit\n\n[[Neighbor]]"),
        ("wiki/concepts/Neighbor.md", "# Neighbor\n\n本地邻接。"),
        ("wiki/concepts/FTS.md", "# FTS\n\nquartz 本文关键词"),
        ("wiki/concepts/misleading.md", "# api\n\n无正文命中。"),
        ("wiki/concepts/孤页.md", "# 孤页\n\n无正文命中。"),
        ("wiki/index.md", "# 索引\n\n[[Explicit]] [[孤页]]"),
    ];
    for (path, body) in pages {
        wiki_page(&mut storage, path, body);
    }
    let schema = format!(
        "# SCHEMA\n\n## 全局\n适用所有页面\n\n## concept\n概念规则\n\n## decision\n{}",
        "无关内容".repeat(1500)
    );
    fs::write(dir.0.join("SCHEMA.md"), schema).unwrap();
    let sources = [Source {
        title: "路由".into(),
        path: "raw/test.md".into(),
        text: "请看网关的重试机制；[[Explicit]] quartz capital".into(),
    }];
    let context = storage.prepare_compilation(&sources).unwrap();
    let paths: Vec<_> = context
        .candidates
        .iter()
        .map(|&id| context.catalog.docs[id].path.as_str())
        .collect();
    assert_eq!(paths[0], "wiki/concepts/Explicit.md");
    for path in [
        "wiki/concepts/网关.md",
        "wiki/concepts/gateway.md",
        "wiki/concepts/FTS.md",
        "wiki/concepts/Neighbor.md",
    ] {
        assert!(paths.contains(&path), "{paths:?}");
    }
    for path in [
        "wiki/concepts/misleading.md",
        "wiki/concepts/孤页.md",
        "wiki/index.md",
    ] {
        assert!(!paths.contains(&path), "{paths:?}");
    }
    assert!(!context.single_step);
    assert!(context.input["index_map"].as_str().unwrap().lines().count() <= 12);
    let schema = context.input["schema"].as_str().unwrap();
    assert!(schema.contains("概念规则") && schema.contains("适用所有页面"));
    assert!(!schema.contains("无关内容"));
}

#[test]
fn compiles_multiple_pages_with_verified_claims_review_and_child_revert() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let a = "wiki/concepts/主页面.md";
    let b = "wiki/entities/实体页.md";
    let c = "wiki/decisions/决策页.md";
    let original_c = "# 决策页\r\n\r\n## 策略\r\n\r\n旧决策\r\n\r\n%%旧注释保留%%\r\n";
    wiki_page(&mut storage, a, "# 主页面\n\n## 简介\n人工原文\n");
    wiki_page(&mut storage, b, "# 实体页\n");
    wiki_page(&mut storage, c, original_c);
    let input = clip(
        &mut storage,
        "[[主页面]] [[实体页]] [[决策页]]\r\n功能  保持稳定；窗口使用本地规则。",
    );
    let context = storage.prepare_compilation(&input.sources).unwrap();
    let (ca, cb, cc) = (
        candidate(&context, a),
        candidate(&context, b),
        candidate(&context, c),
    );
    let heading = context.catalog.docs[context.catalog.paths[&c.to_lowercase()]]
        .parsed
        .headings
        .iter()
        .filter(|h| h.level == 2)
        .position(|h| h.title == "策略")
        .unwrap()
        + 1;
    let body = json!({
        "summary":"跨页面整理",
        "routes":[
            {"target":ca,"units":["s1"],"relation":"extends"},
            {"target":cb,"units":["s1"],"relation":"extends"},
            {"target":cc,"units":["s1"],"relation":"conflicts","why":"解释存在差异"},
            {"target":{"new":{"type":"concept","title":"薄页"}},"units":["s1"],"relation":"extends"},
            {"target":{"new":{"type":"entity","title":"无据页"}},"units":["s1"],"relation":"extends"}
        ],
        "ops":[
            {"op":"append_section","page":"p1","heading":"简介","claims":[
                {"text":"可用 [[实体页]]、[[不存在]]，不收假脚注[^fake]","source":"s1","quote":"功能 保持稳定"},
                {"text":"假的结论","source":"s1","quote":"缺失"}]},
            {"op":"append_section","page":"p2","heading":"背景","claims":[{"text":"补充实体","source":"s1","quote":"窗口使用本地规则。"}]},
            {"op":"replace_section","page":"p3","section":format!("h{heading}"),"claims":[{"text":"更新策略","source":"s1","quote":"窗口使用本地规则。"}]},
            {"op":"create_page","page":"n1","lead":"已有依据","sections":[{"heading":"事实","claims":[{"text":"稳定","source":"s1","quote":"功能 保持稳定"}]}]},
            {"op":"create_page","page":"n2","lead":"","sections":[{"heading":"事实","claims":[{"text":"证据不在场","source":"s1","quote":"无依据"}]}]},
            {"op":"add_link","page":"p1","to":"n2","context":"应被连带丢弃"}
        ]
    });
    let result = fixture_output(context, &input.sources, body);
    let compiled = storage
        .apply_inbox_compilation(&input, "fixture", result)
        .unwrap();
    let summary = compiled.compile_summary.unwrap();
    assert_eq!(
        (
            summary.applied,
            summary.pending,
            summary.dropped,
            summary.remaining_paragraphs
        ),
        (2, 2, 2, 0)
    );
    let id = &summary.changeset_id;
    let detail = storage.get_changeset_detail(id).unwrap();
    assert_eq!(detail.changeset.dropped_claims, 2);
    let decision = detail
        .pages
        .iter()
        .find(|page| page.page.path == c)
        .unwrap();
    assert!(decision.has_pending && !decision.externally_modified);
    assert!(decision.after.as_ref().unwrap().contains("%%旧注释保留%%"));
    assert_eq!(decision.before.as_deref(), Some(original_c));
    assert_eq!(fs::read_to_string(dir.0.join(c)).unwrap(), original_c);
    assert_eq!(
        storage
            .list_conflicts()
            .unwrap()
            .iter()
            .filter(|c| c.status == "open")
            .count(),
        1
    );
    assert!(!dir.0.join("wiki/entities/无据页.md").exists());
    let result = fs::read_to_string(dir.0.join(a)).unwrap();
    assert!(result.contains("[[实体页]]") && result.contains("、不存在"));
    assert!(
        !result.contains("[[不存在]]") && !result.contains("[^fake]") && !result.contains("&#91;")
    );
    let page = storage.read_document_detail(a).unwrap();
    let citation = storage
        .read_citation_context(a, &page.citations[0].ref_label)
        .unwrap();
    assert!(citation.verified);
    assert_eq!(citation.quote, "功能  保持稳定");

    fs::write(dir.0.join(c), format!("{original_c}\r\n人工更新")).unwrap();
    assert!(
        storage
            .get_changeset_detail(id)
            .unwrap()
            .pages
            .iter()
            .find(|page| page.page.path == c)
            .unwrap()
            .externally_modified
    );
    assert_eq!(
        storage
            .decide_review(id, c, ReviewDecision::Apply)
            .err()
            .unwrap()
            .code,
        "VERSION_CONFLICT"
    );
    fs::write(dir.0.join(c), original_c).unwrap();
    let parent = storage.decide_review(id, c, ReviewDecision::Apply).unwrap();
    let child = parent
        .ops
        .iter()
        .find(|op| op.target_path == c)
        .unwrap()
        .child_changeset_id
        .as_ref()
        .unwrap();
    assert_eq!(
        storage
            .get_changeset_detail(child)
            .unwrap()
            .changeset
            .parent_id
            .as_deref(),
        Some(id.as_str())
    );
    assert!(
        storage
            .list_conflicts()
            .unwrap()
            .iter()
            .all(|c| c.status != "open")
    );
    let decision = fs::read_to_string(dir.0.join(c)).unwrap();
    assert!(
        decision.contains("更新策略")
            && !decision.contains("wikra_pending_conflicts")
            && decision.contains("%%旧注释保留%%")
    );
    assert_eq!(
        decision.matches('\n').count(),
        decision.matches("\r\n").count()
    );
    assert!(storage.list_wiki_suggestions().unwrap().is_empty());
    assert_eq!(
        storage.revert_changeset(id).err().unwrap().code,
        "VERSION_CONFLICT"
    );
    storage.revert_changeset(child).unwrap();
    assert_eq!(fs::read_to_string(dir.0.join(c)).unwrap(), original_c);
    let discarded = storage
        .decide_review(id, "wiki/concepts/薄页.md", ReviewDecision::Discard)
        .unwrap();
    assert_eq!(discarded.status, "applied");
    assert_eq!(
        discarded
            .ops
            .iter()
            .find(|op| op.target_title == "薄页")
            .unwrap()
            .outcome,
        "discarded"
    );
    assert_eq!(
        storage.revert_changeset(id).unwrap().changeset.status,
        "reverted"
    );
    assert!(!storage.inbox_item(&input.id).unwrap().paragraphs[0].compiled);
}

#[test]
fn keeps_manual_changes_when_reverting_three_pages_and_rebuilds_the_index() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let index = "# 人工索引\r\n\r\n不可移动的文字。\r\n";
    wiki_page(&mut storage, "wiki/index.md", index);
    wiki_page(&mut storage, "wiki/log.md", "# 人工日志\r\n保留  \r\n");
    let page = "wiki/concepts/项目.md";
    let orphan = "wiki/concepts/孤页.md";
    let before = "# 项目\n\n既有内容\n";
    wiki_page(&mut storage, page, before);
    wiki_page(
        &mut storage,
        orphan,
        "---\ncreated: 2020-02-01\n---\n# 孤页\n",
    );
    let input = clip(&mut storage, "[[项目]] 一期原句，二期原句，三期原句。");
    let context = storage.prepare_compilation(&input.sources).unwrap();
    let target = candidate(&context, page);
    let mut routes = vec![json!({"units":["s1"],"target":target,"relation":"extends"})];
    let mut edits = vec![
        json!({"op":"append_section","page":"p1","heading":"进展","claims":[{"source":"s1","quote":"一期原句","text":"一期"}]}),
    ];
    for (ordinal, title, quote) in [(1, "二期", "二期原句"), (2, "三期", "三期原句")] {
        routes.push(json!({"units":["s1"],"target":{"new":{"type":"entity","title":title}},"relation":"different_scope"}));
        edits.push(json!({"op":"create_page","page":format!("n{ordinal}"),"lead":"","sections":[{"heading":"事实","claims":[{"source":"s1","quote":quote,"text":title}]}]}));
    }
    let output = fixture_output(
        context,
        &input.sources,
        json!({"routes":routes,"ops":edits,"summary":"多页回滚"}),
    );
    let item = storage
        .apply_inbox_compilation(&input, "fixture", output)
        .unwrap();
    let id = item.compile_summary.unwrap().changeset_id;
    assert_eq!(
        storage
            .get_changeset_detail(&id)
            .unwrap()
            .changeset
            .pages
            .len(),
        3
    );
    let edited = "wiki/entities/二期.md";
    let keep = fs::read_to_string(dir.0.join(edited)).unwrap();
    fs::write(dir.0.join(edited), format!("{keep}\n人工补充")).unwrap();
    let log = fs::read_to_string(dir.0.join("wiki/log.md")).unwrap();
    assert!(log.starts_with("# 人工日志\r\n保留  \r\n") && log.contains("[compile]"));
    let reverted = storage.revert_changeset(&id).unwrap();
    assert_eq!(reverted.changeset.status, "partially_reverted");
    assert_eq!(reverted.restored.len(), 2);
    assert_eq!(reverted.kept_modified, [edited]);
    assert_eq!(fs::read_to_string(dir.0.join(page)).unwrap(), before);
    assert!(!dir.0.join("wiki/entities/三期.md").exists());
    assert!(storage.inbox_item(&input.id).unwrap().paragraphs[0].compiled);
    let after = fs::read_to_string(dir.0.join("wiki/index.md")).unwrap();
    assert!(after.starts_with(index) && after.contains("二期") && !after.contains("三期"));
    let after_log = fs::read_to_string(dir.0.join("wiki/log.md")).unwrap();
    assert!(after_log.starts_with(&log) && after_log.contains("[revert]"));
    for include_meta in [false, true] {
        let graph = storage.get_knowledge_graph(include_meta).unwrap();
        assert!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == orphan)
                .unwrap()
                .is_orphan
        );
        assert!(!graph.edges.iter().any(|edge| edge.from == "wiki/index.md"));
    }
    fs::write(dir.0.join(edited), keep).unwrap();
    let full = storage.revert_changeset(&id).unwrap();
    assert_eq!(full.changeset.status, "reverted");
    assert!(!storage.inbox_item(&input.id).unwrap().paragraphs[0].compiled);
}

#[test]
fn caps_source_material_without_marking_the_remaining_paragraphs() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let first = format!("现有结论是事实。{}", "甲".repeat(11_985));
    let original = format!("{first}\n\n这里是下次整理的段落。");
    let input = clip(&mut storage, &original);
    let context = storage.prepare_compilation(&input.sources).unwrap();
    assert_eq!(context.source_count, 1);
    assert!(context.single_step);
    assert_eq!(context.input["sources"].as_array().unwrap().len(), 1);
    let response =
        json!({"noop":true,"noopReason":"已涵盖","routes":[],"summary":"本批无需追加","ops":[]});
    let output = fixture_output(context, &input.sources, response);
    let result = storage
        .apply_inbox_compilation(&input, "fixture", output)
        .unwrap();
    assert_eq!(result.compile_summary.unwrap().remaining_paragraphs, 1);
    assert!(result.paragraphs[0].compiled && !result.paragraphs[1].compiled);
    assert_eq!(
        storage
            .prepare_inbox_compile(&input.id, CompileMode::Full)
            .unwrap()
            .unwrap()
            .sources[0]
            .text,
        "这里是下次整理的段落。"
    );
}

#[test]
fn clip_title_can_become_a_wiki_title_and_repeated_claims_do_not_write_again() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let input = clip(&mut storage, "事实原句");
    let path = "wiki/entities/整理样本.md";
    let response = || {
        json!({
            "summary":"单条证据",
            "routes":[{"target":{"new":{"type":"entity","title":"整理样本"}},"units":["s1"],"relation":"extends"}],
            "ops":[{"op":"create_page","page":"n1","lead":"","sections":[{"heading":"结论","claims":[
                {"text":"已确认的事实","source":"s1","quote":"事实原句"},
                {"text":"缺少短引字段的主张","source":"s1"}
            ]}]}]
        })
    };
    let context = storage.prepare_compilation(&input.sources).unwrap();
    let output = fixture_output(context, &input.sources, response());
    let first = storage
        .apply_inbox_compilation(&input, "fixture", output)
        .unwrap();
    assert_eq!(first.related_page_paths, [path]);
    let first_body = fs::read_to_string(dir.0.join(path)).unwrap();
    assert!(first_body.contains("已确认的事实"));
    assert!(!first_body.contains("缺少短引"));
    assert_eq!(
        storage.resolve_wikilink("", "整理样本").unwrap().matches[0].path,
        path
    );

    let repeated = clip(&mut storage, "事实原句");
    let context = storage.prepare_compilation(&repeated.sources).unwrap();
    let output = fixture_output(context, &repeated.sources, response());
    let result = storage
        .apply_inbox_compilation(&repeated, "fixture", output)
        .unwrap();
    let batch = storage
        .get_changeset_detail(&result.compile_summary.unwrap().changeset_id)
        .unwrap();
    assert_eq!(batch.changeset.ops[0].op, "append_section");
    assert_eq!(batch.changeset.dropped_claims, 1);
    assert!(batch.changeset.pages.is_empty());
    assert_eq!(fs::read_to_string(dir.0.join(path)).unwrap(), first_body);
}

#[test]
fn reviews_duplicate_headings_and_racing_edits_and_drops_protected_or_unsourced_replacements() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let dup = "wiki/Same.md";
    let edited = "wiki/Edited.md";
    let locked = "wiki/Locked.md";
    let no_quote = "wiki/Strict.md";
    let old = "# Edited\n\n## Update\n\nOld content\n";
    wiki_page(
        &mut storage,
        dup,
        "# Same\n\n## Update\n\nOne\n\n## Update\n\nTwo",
    );
    wiki_page(&mut storage, edited, old);
    let protected = "# Locked\n\n<!-- wikra-protect:start -->\n## Update\n\nKeep the exact original.\n<!-- wikra-protect:end -->";
    wiki_page(&mut storage, locked, protected);
    wiki_page(
        &mut storage,
        no_quote,
        "# Strict\n\n## Update\n\nKeep everything here.\n",
    );
    let input = clip(
        &mut storage,
        "[[Same]] [[Edited]] [[Locked]] [[Strict]]\n真实依据",
    );
    let context = storage.prepare_compilation(&input.sources).unwrap();
    let routes: Vec<_> = [dup, edited, locked, no_quote]
        .into_iter()
        .map(|path| json!({"target":candidate(&context,path),"units":["s1"],"relation":"extends"}))
        .collect();
    let claim = json!({"source":"s1","quote":"真实依据","text":"新结论"});
    let output = fixture_output(
        context,
        &input.sources,
        json!({"routes":routes,"summary":"审阅",
        "ops":[
            {"op":"append_section","page":"p1","heading":"Update","claims":[claim]},
            {"op":"append_section","page":"p2","heading":"Update","claims":[claim]},
            {"op":"append_section","page":"p3","heading":"Update","claims":[claim]},
            {"op":"replace_section","page":"p4","section":"h1","claims":[claim,{"source":"s1","quote":"无原文","text":"假结论"}]}
        ]}),
    );
    fs::write(dir.0.join(edited), format!("{old}\nManual changes.")).unwrap();
    let applied = storage
        .apply_inbox_compilation(&input, "fixture", output)
        .unwrap();
    let summary = applied.compile_summary.unwrap();
    assert_eq!(
        (summary.applied, summary.pending, summary.dropped),
        (0, 2, 2)
    );
    let id = summary.changeset_id;
    let detail = storage.get_changeset_detail(&id).unwrap();
    assert!(
        detail
            .pages
            .iter()
            .any(|page| page.page.path == edited && page.externally_modified)
    );
    assert!(
        detail.pages.iter().any(|page| page.page.path == dup
            && page.after.as_ref().unwrap().contains("## Update（补充）"))
    );
    assert_eq!(fs::read_to_string(dir.0.join(locked)).unwrap(), protected);
    assert!(
        fs::read_to_string(dir.0.join(no_quote))
            .unwrap()
            .contains("Keep everything here.")
    );
    storage
        .decide_review(&id, dup, ReviewDecision::Apply)
        .unwrap();
    assert!(
        fs::read_to_string(dir.0.join(dup))
            .unwrap()
            .contains("## Update（补充）")
    );
}

#[test]
fn structure_lint_fixes_real_issues_and_ignores_comments_and_code() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    let path = "wiki/concepts/Report.md";
    let body = "---\r\ncreated: 2020-01-01\r\nwikra_pending_conflicts: [closed] # user comment\r\ncustom: keep\r\n---\r\n\
# Report\r\n\r\n[[Old/Operations]] [[#missing-section]] [[Shared]]\r\n<!-- [[HtmlNever]] -->\r\n\
%% [[CommentNever]] %%\r\n`[[CodeNever]]`\r\n```text\r\n[[FenceNever]]\r\n```\r\n\
[^missing]: [source](<raw/missing.md>) —— “人工短引”\r\n[^mismatch]: [source](<raw/source.md>) —— “原文不存在的字”\r\n";
    fs::write(dir.0.join("raw/source.md"), "短引正本").unwrap();
    wiki_page(&mut storage, path, body);
    wiki_page(
        &mut storage,
        "wiki/concepts/Operations.md",
        "# Operations\n",
    );
    wiki_page(
        &mut storage,
        "wiki/concepts/A.md",
        "---\naliases: [Shared]\n---\n# A\n",
    );
    wiki_page(
        &mut storage,
        "wiki/entities/B.md",
        "---\naliases: [Shared]\n---\n# B\n",
    );
    wiki_page(
        &mut storage,
        "wiki/entities/Old.md",
        "---\ncreated: 2020-01-01\n---\n# Old\n",
    );
    wiki_page(
        &mut storage,
        "wiki/entities/Unclosed.md",
        "# Unclosed\n\n<!-- fmd-protect:start -->\n",
    );
    let report = storage.run_structure_lint().unwrap();
    let rules: HashSet<_> = report
        .findings
        .iter()
        .map(|finding| finding.rule.as_str())
        .collect();
    for expected in [
        "broken_link",
        "broken_anchor",
        "ambiguous_name",
        "missing_source",
        "quote_mismatch",
        "stale_conflict_marker",
        "structure_blocked",
        "index_drift",
        "orphan",
        "thin_page",
    ] {
        assert!(rules.contains(expected), "{rules:?}");
    }
    assert!(
        report
            .findings
            .iter()
            .all(|finding| !finding.message.contains("Never"))
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.rule == "orphan" && finding.page_title == "Operations")
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| finding.rule == "index_drift")
            .count(),
        1
    );
    let id = report
        .findings
        .iter()
        .find(|finding| finding.rule == "broken_link")
        .unwrap()
        .id
        .clone();
    storage.dismiss_suggestion(&id).unwrap();
    let updated = body.replace("# Report\r\n\r\n", "# Report\r\n\r\n\r\n");
    fs::write(dir.0.join(path), &updated).unwrap();
    assert!(
        storage
            .run_structure_lint()
            .unwrap()
            .findings
            .iter()
            .any(|finding| finding.id == id && finding.dismissed)
    );
    storage.undismiss_suggestion(&id).unwrap();
    let report = storage.run_structure_lint().unwrap();
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.id == id && !finding.dismissed)
    );
    let fixes: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.fix.is_some())
        .map(|finding| finding.id.clone())
        .collect();
    let result = storage.apply_lint_fix(&fixes).unwrap();
    assert!(result.index_rebuilt);
    let fixed = result.changeset.unwrap();
    assert_eq!(fixed.pages.len(), 1);
    let content = fs::read_to_string(dir.0.join(path)).unwrap();
    assert!(content.contains("[[wiki/concepts/Operations|Operations]]"));
    assert!(content.contains("wikra_pending_conflicts: [] # user comment\r\ncustom: keep\r\n"));
    assert!(!result.report.findings.iter().any(|finding| matches!(
        finding.rule.as_str(),
        "broken_link" | "index_drift" | "stale_conflict_marker"
    )));
    storage.revert_changeset(&fixed.id).unwrap();
    assert_eq!(fs::read_to_string(dir.0.join(path)).unwrap(), updated);
}

#[test]
fn completion_uses_shortest_unique_path_and_resolves_headings_and_blocks() {
    let dir = TestDir::new();
    let mut storage = standard_vault(&dir);
    wiki_page(
        &mut storage,
        "wiki/a/Name.md",
        "---\nstatus: active\naliases: [另名]\nsupersedes: [Old]\n---\n# 正式名称\n\n###### Sixth Heading\n\n内容 ^anchor",
    );
    wiki_page(&mut storage, "wiki/b/Name.md", "# 第二篇\n");
    wiki_page(&mut storage, "wiki/SKILL.md", "# Meta\n\n[[第二篇]]");
    let candidates = storage.list_wikilink_candidates().unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].link_text, "[[a/Name|正式名称]]");
    assert!(candidates[0].ambiguous_stem);
    assert_eq!(
        storage.resolve_wikilink("", "Name").unwrap().status,
        "ambiguous"
    );
    for target in [
        "a/Name#sixth-heading",
        "a/Name#^anchor",
        "a/Name^anchor",
        "另名",
    ] {
        assert_eq!(
            storage.resolve_wikilink("", target).unwrap().status,
            "unique",
            "{target}"
        );
    }
    assert_eq!(
        storage
            .resolve_wikilink("wiki/a/Name.md", "#Sixth Heading")
            .unwrap()
            .status,
        "unique"
    );
    assert_eq!(
        storage
            .resolve_wikilink("", "a/Name#missing")
            .unwrap()
            .status,
        "missing"
    );
    let summary = storage
        .list_library_tree()
        .unwrap()
        .wiki_pages
        .into_iter()
        .find(|page| page.path == "wiki/a/Name.md")
        .unwrap();
    assert_eq!(summary.status.as_deref(), Some("active"));
    assert_eq!(summary.aliases, ["另名"]);
    assert_eq!(
        storage
            .read_document_detail("wiki/a/Name.md")
            .unwrap()
            .supersedes,
        ["Old"]
    );
}

#[test]
fn exports_skill_once_preserving_bom_crlf_and_validates_allowed_raw_folders() {
    let dir = TestDir::new();
    let storage = standard_vault(&dir);
    fs::create_dir(dir.0.join("raw/notes/shared")).unwrap();
    fs::write(dir.0.join("raw/notes/shared/demo.md"), "允许目录").unwrap();
    let original = "\u{feff}---\r\ncustom: 1 # 保留\r\n---\r\n# 用户手记\r\n";
    let skill = dir.0.join("SKILL.md");
    fs::write(&skill, original).unwrap();
    assert_eq!(storage.export_vault_skill().unwrap().action, "updated");
    let first = fs::read_to_string(&skill).unwrap();
    let mtime = fs::metadata(&skill).unwrap().modified().unwrap();
    assert_eq!(storage.export_vault_skill().unwrap().action, "unchanged");
    assert_eq!(fs::metadata(&skill).unwrap().modified().unwrap(), mtime);
    assert!(first.starts_with(&original.replace("# 保留\r\n", "# 保留\r\nname: wikra-lookup\r\n")));
    assert_eq!(first.matches('\n').count(), first.matches("\r\n").count());
    assert_eq!(
        storage
            .set_mcp_raw_allow(&["raw/notes/shared".into()])
            .unwrap(),
        ["raw/notes/shared"]
    );
    for folder in [".wikra", "wiki", "raw/../wiki", "raw_backup", ".."] {
        assert!(
            storage.set_mcp_raw_allow(&[folder.into()]).is_err(),
            "{folder}"
        );
    }
    let config = storage.get_agent_integration().unwrap();
    assert_eq!(config.raw_allow, ["raw/notes/shared"]);
    assert!(
        config
            .raw_dirs
            .iter()
            .any(|folder| folder.path == "raw/notes/shared"
                && folder.file_count == 1
                && folder.is_capture_dir)
    );
    let snippets: Value = serde_json::to_value(config.snippets).unwrap();
    assert_eq!(
        snippets.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["claudeCli", "codexToml", "mcpJson"]
    );
    assert!(
        Catalog::load(&storage)
            .unwrap()
            .docs
            .iter()
            .find(|doc| doc.path == "SKILL.md")
            .unwrap()
            .meta()
    );
}
