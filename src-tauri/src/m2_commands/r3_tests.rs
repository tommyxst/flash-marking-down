use std::fs;
use std::net::TcpListener;
use std::path::Path;

use crate::memory::tests::{mock_http, mock_http_response};
use crate::storage::{SavedRecord, tests::TestDir};

use super::*;

const HISTORY_URI: &str = "viking://user/fixture/memories/entities/quartz.md";

fn save_fixture(storage: &mut Storage, revision: i64, body: &str) -> SavedRecord {
    storage.save_draft("capture", revision, body).unwrap();
    storage
        .save_record(
            &format!("1795579200000-{revision}"),
            "capture",
            revision,
            body,
        )
        .unwrap()
}

fn configure_memory(storage: &Storage, base_url: &str) {
    let config = MemoryConfig::new(base_url, "r3-test", true, true, true).unwrap();
    storage.save_service_config("memory", &config, "").unwrap();
}

fn unavailable_address() -> tauri::Url {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    tauri::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap()
}

#[test]
fn edited_capture_runs_organize_and_both_sync_lanes_on_the_new_body() {
    let dir = TestDir::new();
    let mut storage = Storage::open(&dir.0, Path::new("."), ".state").unwrap();
    let versions = [
        ("原始笔记", "原始整理摘要", "核对旧资料"),
        ("修改后的笔记", "新版整理摘要", "跟进新资料"),
    ];
    let mut calls = 0;
    let (url, server) = mock_http(12, move |request| {
        let (title, summary, task) = versions[calls / 6];
        let expected = [
            "/api/v1/content/write",
            "/api/v1/search/search",
            "/v1/chat/completions",
            "/api/v1/sessions",
            "/api/v1/sessions/isolated/messages",
            "/api/v1/sessions/isolated/commit",
        ];
        assert_eq!(request.target, expected[calls % 6]);
        calls += 1;
        let body = request.body.unwrap();
        let result = match request.target.as_str() {
            "/api/v1/content/write" => {
                let content = body["content"].as_str().unwrap();
                assert!(content.contains(title) && content.contains("同日相邻随手记"));
                if calls > 6 {
                    assert!(!content.contains("原始笔记"));
                }
                json!({"uri": body["uri"]})
            }
            "/api/v1/search/search" => json!({"memories": []}),
            "/v1/chat/completions" => {
                let input = body["messages"][1]["content"].as_str().unwrap();
                assert!(input.contains(title) && input.contains(task));
                let organized = json!({"summary": summary, "tasks": [{"text": task, "line": 2}]});
                return (
                    200,
                    json!({"choices":[{"message":{"content": organized.to_string()}, "finish_reason":"stop"}]}),
                );
            }
            "/api/v1/sessions" => json!({"session_id": "isolated"}),
            "/api/v1/sessions/isolated/messages" => {
                let content = body["content"].as_str().unwrap();
                assert!(
                    content.contains(title) && content.contains(summary) && content.contains(task)
                );
                json!({})
            }
            _ => json!({"task_id": "fixture-task"}),
        };
        (200, json!({"status": "ok", "result": result}))
    });
    configure_memory(&storage, url.as_str());
    storage
        .set_model_config(url.as_str().into(), "fixture".into(), true)
        .unwrap();
    let prefs = AppSettings::default();
    let (old_title, old_summary, old_task) = versions[0];
    let first = save_fixture(&mut storage, 1, &format!("{old_title}\n- [ ] {old_task}"));
    let adjacent = save_fixture(&mut storage, 2, "同日相邻随手记");
    assert!(queue_saved_note(&mut storage, &first.record_id, &prefs));
    for organized in [false, true, false] {
        assert_eq!(
            run_background_job(&mut storage, true, |_| {}).unwrap(),
            Some((first.record_id.clone(), organized))
        );
    }
    let detail = storage.get_note_detail(&first.record_id).unwrap();
    assert_eq!(detail.ov_resource_status, "synced");
    assert_eq!(detail.ov_memory_status, "synced");
    assert_eq!(
        storage
            .get_note_detail(&adjacent.record_id)
            .unwrap()
            .ov_resource_status,
        "synced"
    );
    assert_eq!(detail.organize.as_ref().unwrap().summary, old_summary);

    assert!(storage.enqueue_organize_job(&first.record_id).unwrap());
    let running = storage.next_background_job(None).unwrap().unwrap();
    let old_input = storage.prepare_organize_job(&running).unwrap();
    let (new_title, new_summary, new_task) = versions[1];
    let (updated, wake) = update_note_and_queue(
        &mut storage,
        &detail.locator,
        &detail.content_hash,
        &format!("{new_title}\n- [ ] {new_task}"),
        true,
        &prefs,
    )
    .unwrap();
    assert!(wake);
    assert_eq!(updated.locator, detail.locator);
    assert_eq!(updated.title, new_title);
    assert_eq!(updated.organize_status, "queued");
    assert_eq!(updated.ov_resource_status, "pending");
    assert_eq!(updated.ov_memory_status, "pending");
    let old_result = updated.organize.unwrap();
    assert!(old_result.source_changed);
    assert_eq!(old_result.summary, old_summary);
    assert_eq!(
        storage
            .get_note_detail(&adjacent.record_id)
            .unwrap()
            .ov_resource_status,
        "pending"
    );

    storage
        .complete_organize_job(&old_input, Ok(json!({"summary": "在途任务返回的旧摘要"})))
        .unwrap();
    let pending = storage.get_note_detail(&detail.locator).unwrap();
    assert_eq!(pending.organize_status, "queued");
    assert!(pending.organize.unwrap().source_changed);

    for organized in [false, true, false] {
        assert_eq!(
            run_background_job(&mut storage, true, |_| {}).unwrap(),
            Some((first.record_id.clone(), organized))
        );
    }
    assert!(
        run_background_job(&mut storage, true, |_| {})
            .unwrap()
            .is_none()
    );
    server.join().unwrap();
    let done = storage.get_note_detail(&detail.locator).unwrap();
    assert_eq!(done.organize_status, "done");
    assert_eq!(done.ov_resource_status, "synced");
    assert_eq!(done.ov_memory_status, "synced");
    let organized = done.organize.unwrap();
    assert!(!organized.source_changed);
    assert_eq!(organized.summary, new_summary);
    assert_eq!(organized.tasks.len(), 1);
    assert_eq!(organized.tasks[0].text, new_task);
}

#[test]
fn edit_during_sync_keeps_newer_resource_and_memory_work_pending() {
    let dir = TestDir::new();
    let mut storage = Storage::open(&dir.0, Path::new("."), ".state").unwrap();
    configure_memory(&storage, "http://127.0.0.1:1933");
    let config = memory_config(&storage).unwrap().unwrap();
    let prefs = AppSettings::default();
    let saved = save_fixture(&mut storage, 1, "最初正文");
    assert!(queue_saved_note(&mut storage, &saved.record_id, &prefs));
    let before = storage.get_note_detail(&saved.record_id).unwrap();
    let (through_row, _) = storage.resource_batch(&saved.relative_path).unwrap();
    let source = storage.read_note_markdown(&saved.relative_path).unwrap();
    let resource = storage.next_background_job(Some(&config)).unwrap().unwrap();
    assert_eq!(resource.kind, "ov_write_resource");

    let (middle, wake) = update_note_and_queue(
        &mut storage,
        &before.locator,
        &before.content_hash,
        "资源在途时的修改",
        true,
        &prefs,
    )
    .unwrap();
    assert!(wake);
    storage
        .complete_resource_job(
            &resource,
            through_row,
            Some(&source),
            Err("旧请求超时".into()),
        )
        .unwrap();
    assert_eq!(
        storage
            .get_note_detail(&before.locator)
            .unwrap()
            .ov_resource_status,
        "pending"
    );
    let replacement = storage.next_background_job(Some(&config)).unwrap().unwrap();
    assert_eq!(replacement.kind, "ov_write_resource");
    assert_ne!(replacement.id, resource.id);
    let source = storage.read_note_markdown(&saved.relative_path).unwrap();
    storage
        .complete_resource_job(
            &replacement,
            through_row,
            Some(&source),
            Ok("fixture-uri".into()),
        )
        .unwrap();

    let memory = storage.next_background_job(Some(&config)).unwrap().unwrap();
    assert_eq!(memory.kind, "ov_commit_memory");
    let (_, wake) = update_note_and_queue(
        &mut storage,
        &middle.locator,
        &middle.content_hash,
        "记忆在途时的修改",
        true,
        &prefs,
    )
    .unwrap();
    assert!(wake);
    storage
        .complete_memory_job(&memory, Ok(Some("old-task".into())))
        .unwrap();
    let updated = storage.get_note_detail(&middle.locator).unwrap();
    assert_eq!(updated.ov_resource_status, "pending");
    assert_eq!(updated.ov_memory_status, "pending");
    let follow_ups = ["ov_write_resource", "ov_commit_memory"].map(|kind| {
        let job = storage.next_background_job(Some(&config)).unwrap().unwrap();
        assert_eq!(job.kind, kind);
        job.id
    });
    assert_ne!(follow_ups[1], memory.id);
    assert!(
        storage
            .next_background_job(Some(&config))
            .unwrap()
            .is_none()
    );
}

#[test]
fn wiki_edit_without_reorganize_only_syncs_an_existing_note() {
    let dir = TestDir::new();
    let mut storage = Storage::open(&dir.0, Path::new("."), ".state").unwrap();
    configure_memory(&storage, "http://127.0.0.1:1933");
    storage
        .set_model_config("http://127.0.0.1:11434".into(), "fixture".into(), true)
        .unwrap();
    let config = memory_config(&storage).unwrap().unwrap();
    let prefs = AppSettings::default();
    let path = "wiki.md";
    fs::write(dir.0.join(path), "原始文档\n").unwrap();
    storage
        .search_reader()
        .unwrap()
        .search_records("原始文档", None, None, 0)
        .unwrap();
    let original = storage.get_note_detail(path).unwrap();
    let (updated, wake) = update_note_and_queue(
        &mut storage,
        path,
        &original.content_hash,
        "改写文档",
        false,
        &prefs,
    )
    .unwrap();
    assert!(!wake);
    assert_eq!(updated.id, original.id);
    assert!(
        storage
            .next_background_job(Some(&config))
            .unwrap()
            .is_none()
    );

    let manual = storage.ensure_note_for_source(path).unwrap();
    let (updated, wake) = update_note_and_queue(
        &mut storage,
        path,
        &updated.content_hash,
        "继续编辑文档",
        false,
        &prefs,
    )
    .unwrap();
    assert!(wake);
    assert!(!updated.is_capture);
    assert_eq!(updated.id, manual);
    for kind in ["ov_write_resource", "ov_commit_memory"] {
        let job = storage.next_background_job(Some(&config)).unwrap().unwrap();
        assert_eq!(job.kind, kind);
        assert_eq!(job.note_id, manual);
    }
    assert!(
        storage
            .next_background_job(Some(&config))
            .unwrap()
            .is_none()
    );
}

#[test]
fn r3_syncs_daily_file_before_organize_and_memory_and_retries_after_disconnect() {
    let dir = TestDir::new();
    let mut storage = Storage::open(&dir.0, Path::new("."), ".state").unwrap();
    let mut call = 0;
    let (url, server) = mock_http(11, move |request| {
        let expected = [
            "/api/v1/content/write",
            "/api/v1/search/search",
            "/v1/chat/completions",
            "/api/v1/search/search",
            "/v1/chat/completions",
            "/api/v1/sessions",
            "/api/v1/sessions/isolated/messages",
            "/api/v1/sessions/isolated/commit",
            "/api/v1/sessions",
            "/api/v1/sessions/isolated/messages",
            "/api/v1/sessions/isolated/commit",
        ];
        assert_eq!(request.target, expected[call]);
        call += 1;
        let body = request.body.unwrap();
        let result = match request.target.as_str() {
            "/api/v1/content/write" => {
                let content = body["content"].as_str().unwrap();
                assert!(content.contains("合成第一条") && content.contains("合成第二条"));
                assert!(body["uri"].as_str().unwrap().contains("/r3-test/daily/"));
                json!({"uri": body["uri"]})
            }
            "/api/v1/search/search" => {
                assert_eq!(body["limit"], 3);
                json!({"memories": [{"uri": HISTORY_URI, "content": "Quartz 测试历史", "score": 0.91}]})
            }
            "/v1/chat/completions" => {
                let input = body["messages"][1]["content"].as_str().unwrap();
                assert!(input.contains(HISTORY_URI));
                let organized = json!({"summary":"来自整理", "decisions":["保留原方案"], "tasks":[{"text":"检查回环端点"}]});
                return (
                    200,
                    json!({"choices":[{"message":{"content":organized.to_string()},"finish_reason":"stop"}]}),
                );
            }
            "/api/v1/sessions" => json!({"session_id":"isolated"}),
            "/api/v1/sessions/isolated/messages" => {
                let content = body["content"].as_str().unwrap();
                assert!(content.contains("## 摘要\n来自整理"));
                assert!(content.contains("## 决定\n- 保留原方案"));
                assert!(content.contains("## 待办\n- [ ] 检查回环端点"));
                json!({})
            }
            _ => json!({"task_id":"fixture-task"}),
        };
        (200, json!({"status":"ok","result":result}))
    });
    configure_memory(&storage, url.as_str());
    storage
        .set_model_config(url.as_str().into(), "fixture".into(), true)
        .unwrap();
    let prefs = AppSettings::default();
    let first = save_fixture(&mut storage, 1, "合成第一条");
    assert!(queue_saved_note(&mut storage, &first.record_id, &prefs));
    let second = save_fixture(&mut storage, 2, "合成第二条");
    assert!(queue_saved_note(&mut storage, &second.record_id, &prefs));
    assert_eq!(first.relative_path, second.relative_path);
    for _ in 0..5 {
        assert!(
            run_background_job(&mut storage, true, |_| {})
                .unwrap()
                .is_some()
        );
    }
    assert!(
        run_background_job(&mut storage, true, |_| {})
            .unwrap()
            .is_none()
    );
    for id in [&first.record_id, &second.record_id] {
        let detail = storage.get_note_detail(id).unwrap();
        assert_eq!(detail.organize_status, "done");
        assert_eq!(detail.ov_resource_status, "synced");
        assert_eq!(detail.ov_memory_status, "synced");
    }
    server.join().unwrap();

    let (model, model_server) = mock_http(1, |request| {
        let body = request.body.unwrap();
        assert!(
            !body["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains(HISTORY_URI)
        );
        (
            200,
            json!({"choices":[{"message":{"content":"{\"summary\":\"离线时仍完成本地整理\"}"},"finish_reason":"stop"}]}),
        )
    });
    storage
        .set_model_config(model.as_str().into(), "fixture".into(), true)
        .unwrap();
    configure_memory(&storage, unavailable_address().as_str());
    let third = save_fixture(&mut storage, 3, "断连期间保存");
    assert!(queue_saved_note(&mut storage, &third.record_id, &prefs));
    assert!(dir.0.join(&third.relative_path).is_file());
    for _ in 0..3 {
        assert!(
            run_background_job(&mut storage, true, |_| {})
                .unwrap()
                .is_some()
        );
    }
    model_server.join().unwrap();
    let failed = storage.get_note_detail(&third.record_id).unwrap();
    assert_eq!(failed.organize_status, "done");
    assert_eq!(failed.ov_resource_status, "failed");
    assert_eq!(failed.ov_memory_status, "failed");
    assert!(failed.ov_error.is_some());
    assert_eq!(
        storage
            .get_note_detail(&first.record_id)
            .unwrap()
            .ov_resource_status,
        "synced"
    );

    let (restored, restored_server) = mock_http(4, |request| {
        let result = match request.target.as_str() {
            "/api/v1/content/write" => {
                let body = request.body.unwrap();
                assert!(body["content"].as_str().unwrap().contains("断连期间保存"));
                json!({"uri": body["uri"]})
            }
            "/api/v1/sessions" => json!({"session_id":"restored"}),
            "/api/v1/sessions/restored/messages" => json!({}),
            "/api/v1/sessions/restored/commit" => json!({"task_id":"restored-task"}),
            _ => panic!("Unexpected endpoint: {}", request.target),
        };
        (200, json!({"status":"ok","result":result}))
    });
    configure_memory(&storage, restored.as_str());
    assert_eq!(queue_memory_retry(&mut storage, None).unwrap(), 2);
    assert_eq!(
        queue_memory_retry(&mut storage, Some(&failed.locator)).unwrap(),
        0
    );
    for _ in 0..2 {
        assert!(
            run_background_job(&mut storage, true, |_| {})
                .unwrap()
                .is_some()
        );
    }
    restored_server.join().unwrap();
    let retried = storage.get_note_detail(&third.record_id).unwrap();
    assert_eq!(retried.ov_resource_status, "synced");
    assert_eq!(retried.ov_memory_status, "synced");
    assert!(retried.ov_error.is_none());
}

#[test]
fn r3_repushes_the_current_daily_file_after_a_checkbox_changes() {
    let dir = TestDir::new();
    let workspace = Storage::open(&dir.0, Path::new("."), ".state")
        .unwrap()
        .into_workspace();
    let concurrent = workspace.clone();
    workspace
        .with_storage::<_, AppError>(|storage| {
            let first = save_fixture(storage, 1, "- [ ] 回环待办");
            let second = save_fixture(storage, 2, "同日第二条");
            let locator = storage.get_note_detail(&first.record_id).unwrap().locator;
            let concurrent_locator = locator.clone();
            let mut round = 0;
            let daily = format!(
                "viking://resources/flash-marking-down/r3-test/daily/{}",
                first.relative_path.rsplit('/').next().unwrap()
            );
            let (url, server) = mock_http(3, move |request| {
                assert_eq!(request.target, "/api/v1/content/write");
                let body = request.body.unwrap();
                assert_eq!(body["uri"], daily);
                assert_eq!(body["mode"], "replace");
                let markdown = body["content"].as_str().unwrap();
                assert!(markdown.contains("同日第二条"));
                if round == 1 {
                    assert!(markdown.contains("- [x] 回环待办"));
                    concurrent
                        .with_storage::<_, AppError>(|storage| {
                            assert!(storage.toggle_task(
                                &concurrent_locator,
                                "line:1",
                                "- [x] 回环待办",
                                false,
                            )?);
                            Ok(())
                        })
                        .unwrap();
                } else {
                    assert!(markdown.contains("- [ ] 回环待办"));
                }
                round += 1;
                (200, json!({"status":"ok","result":{"uri":body["uri"]}}))
            });
            let config = MemoryConfig::new(url.as_str(), "r3-test", true, true, false).unwrap();
            storage.save_service_config("memory", &config, "").unwrap();
            let assert_status = |storage: &Storage, expected| {
                for id in [&first.record_id, &second.record_id] {
                    let detail = storage.get_note_detail(id).unwrap();
                    assert_eq!(detail.ov_resource_status, expected);
                    assert_eq!(detail.ov_memory_status, "pending");
                }
            };
            assert_eq!(storage.enqueue_memory_sync(&config, None, true)?, 1);
            assert!(run_background_job(storage, true, |_| {})?.is_some());
            assert_status(storage, "synced");

            assert!(storage.toggle_task(&locator, "line:1", "- [ ] 回环待办", true)?);
            assert_status(storage, "pending");
            assert_eq!(storage.enqueue_memory_sync(&config, None, true)?, 1);
            assert!(run_background_job(storage, true, |_| {})?.is_some());
            assert_status(storage, "pending");

            assert_eq!(storage.enqueue_memory_sync(&config, None, true)?, 1);
            assert!(run_background_job(storage, true, |_| {})?.is_some());
            assert_status(storage, "synced");
            assert_eq!(storage.enqueue_memory_sync(&config, None, true)?, 0);
            server.join().unwrap();
            Ok(())
        })
        .unwrap();
}

#[test]
fn r3_combines_deduplicates_and_streams_citations_and_falls_back_to_local() {
    const ARCHIVE_DAY_URI: &str =
        "viking://resources/flash-marking-down/r3-test/daily/2025-01-06.md";
    let dir = TestDir::new();
    fs::create_dir(dir.0.join("inbox")).unwrap();
    fs::create_dir(dir.0.join("archive")).unwrap();
    let mut storage = Storage::open(&dir.0, Path::new("inbox"), ".state").unwrap();
    let saved = save_fixture(&mut storage, 1, "Quartz 本地原文");
    let duplicate = format!(
        "viking://resources/flash-marking-down/r3-test/daily/{}",
        saved.relative_path.rsplit('/').next().unwrap()
    );
    for name in ["2025-01-06", "ref-a", "ref-b", "ref-c", "ref-d"] {
        fs::write(
            dir.0.join("archive").join(format!("{name}.md")),
            "Quartz 本地归档",
        )
        .unwrap();
    }
    storage
        .search_reader()
        .unwrap()
        .rebuild_search_index()
        .unwrap();
    let (url, ov_server) = mock_http(1, move |request| {
        assert_eq!(request.target, "/api/v1/search/search");
        assert_eq!(request.body.unwrap()["limit"], 6);
        (
            200,
            json!({"status":"ok","result":{
                "memories":[{"uri":HISTORY_URI,"content":"Quartz 旧背景","score":0.88}],
                "resources":[
                    {"uri":duplicate,"abstract":"重复的每日文件","score":0.98},
                    {"uri":ARCHIVE_DAY_URI,"abstract":"保留的 OpenViking 原文","score":0.95},
                    {"uri":"viking://resources/fixture/guide.md","abstract":"Quartz 远端手册","score":0.79},
                    {"uri":"viking://resources/fixture/notes.md","abstract":"Quartz 远端笔记","score":0.77},
                    {"uri":"viking://resources/fixture/last.md","abstract":"应截断的召回","score":0.70}
                ]
            }}),
        )
    });
    let mut round = 0;
    let (model, chat_server) = mock_http_response(3, move |request| {
        let body = request.body.unwrap();
        assert_eq!(body["stream"], true);
        let messages = body["messages"].as_array().unwrap();
        let prompt = messages.last().unwrap()["content"].as_str().unwrap();
        assert!(prompt.contains("【本地笔记命中】") && prompt.contains("Quartz 本地原文"));
        assert!(
            prompt.contains("【OpenViking 记忆与资源命中】") && prompt.contains("【相关观点分歧】")
        );
        assert!(!prompt.contains("重复的每日文件"));
        assert!(!prompt.contains("应截断的召回"));
        assert!(prompt.contains("archive/2025-01-06.md"));
        assert_eq!(prompt.contains(ARCHIVE_DAY_URI), round == 0);
        assert_eq!(prompt.contains(HISTORY_URI), round == 0);
        let first_delta = json!({"choices":[{"delta":{"content":"结论 "}}]});
        let citations = if round == 0 { "[1][7][8]" } else { "[1]" };
        let last_delta = json!({"choices":[{"delta":{"content":citations}}]});
        round += 1;
        (
            200,
            "text/event-stream",
            format!("data: {first_delta}\n\ndata: {last_delta}\n\ndata: [DONE]\n\n"),
        )
    });
    let question = validate_chat_question("Quartz").unwrap();
    let offline = unavailable_address();
    for (address, enabled, ov_kinds, notice) in [
        (
            url,
            true,
            &["ov_resource", "ov_memory", "ov_resource", "ov_resource"][..],
            None,
        ),
        (
            offline.clone(),
            true,
            &[][..],
            Some("OpenViking 暂不可用，本次仅检索本地笔记。"),
        ),
        (offline, false, &[][..], None),
    ] {
        let config = MemoryConfig::new(address.as_str(), "r3-test", enabled, true, true).unwrap();
        storage.save_service_config("memory", &config, "").unwrap();
        let mut prepared = storage.prepare_chat_turn(None, question).unwrap();
        prepared.add_openviking_recall(recall_openviking(
            enabled_memory_provider(&storage),
            question,
        ));
        let model_endpoint = crate::model::chat_endpoint(model.as_str()).unwrap();
        let mut chunks = Vec::new();
        let answer = default_http_transport()
            .stream_chat(
                &model_endpoint,
                "fixture",
                None,
                &prepared.model_messages(question),
                &mut |delta| chunks.push(delta.to_owned()),
            )
            .unwrap();
        assert_eq!(chunks.len(), 2);
        let turn = storage
            .commit_chat_turn(&prepared, question, &answer)
            .unwrap();
        let expected: Vec<_> = ["local"; 6]
            .into_iter()
            .chain(ov_kinds.iter().copied())
            .collect();
        assert_eq!(
            turn.assistant_message
                .citations
                .iter()
                .map(|citation| citation.kind.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            turn.assistant_message
                .citations
                .iter()
                .enumerate()
                .all(|(i, c)| c.index == i + 1)
        );
        assert_eq!(turn.assistant_message.notice.as_deref(), notice);
        let prior = storage.get_conversation(&turn.conversation.id).unwrap();
        assert_eq!(prior.messages[1], turn.assistant_message);
    }
    ov_server.join().unwrap();
    chat_server.join().unwrap();
}
