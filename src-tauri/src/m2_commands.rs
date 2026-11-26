use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use tauri::async_runtime::{self, Sender, channel};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::capture_commands::{active_workspace, with_workspace};
use crate::credential_commands::{model_credential_state, model_token, service_token};
use crate::dto::{
    AppError, ChatTurnDto, ConflictDto, ConversationDetailDto, ConversationSummaryDto,
    DrawerCountsDto, MemoryItemDto, ModelConfigDto, NoteDetailDto, NoteSummaryDto, TaskListDto,
    WikiDocumentDto, WikiSuggestionDto, WikiTargetDto,
};
use crate::memory::{MemoryProvider, OpenVikingProvider, markdown_body, memory_item_title};
use crate::model::MemoryConfig;
use crate::platform::default_http_transport;
use crate::settings::{self, AppSettings};
use crate::storage::m2_answer::{OpenVikingRecall, validate_chat_question};
use crate::storage::{Storage, m2::ModelConfig, m2::ORGANIZE_PROMPT};

type CommandResult<T> = Result<T, AppError>;

fn m2_err(stage: &'static str, message: String) -> AppError {
    AppError::from_message("M2_FAILED", stage, true, message)
}

pub(crate) struct OrganizeWorker {
    wake: Sender<()>,
    backfill: AtomicBool,
}

pub(crate) fn init_organize_worker(app: &AppHandle) {
    let (wake, mut rx) = channel::<()>(1);
    app.manage(OrganizeWorker {
        wake,
        backfill: AtomicBool::new(false),
    });
    let handle = app.clone();
    async_runtime::spawn(async move {
        while rx.recv().await.is_some() {
            let mut backfill = handle
                .state::<OrganizeWorker>()
                .backfill
                .swap(false, Ordering::Relaxed);
            loop {
                let app = handle.clone();
                let refill = std::mem::take(&mut backfill);
                let result =
                    async_runtime::spawn_blocking(move || process_background_job(&app, refill))
                        .await;
                match result
                    .unwrap_or_else(|_| Err(m2_err("background_jobs", "后台任务已中断".into())))
                {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(err) => {
                        eprintln!("后台队列出错（{}）", err.safe_message);
                        break;
                    }
                }
            }
        }
    });
}

pub(crate) fn wake_organize_worker(app: &AppHandle) {
    if let Some(worker) = app.try_state::<OrganizeWorker>() {
        let _ = worker.wake.try_send(());
    }
}

pub(crate) fn wake_memory_backfill(app: &AppHandle) {
    if let Some(worker) = app.try_state::<OrganizeWorker>() {
        worker.backfill.store(true, Ordering::Relaxed);
        let _ = worker.wake.try_send(());
    }
}

fn memory_config(storage: &Storage) -> CommandResult<Option<MemoryConfig>> {
    storage
        .service_config("memory")
        .map_err(|msg| m2_err("memory_config", msg))
}

fn memory_access(
    storage: &Storage,
    config: &MemoryConfig,
) -> CommandResult<(tauri::Url, Option<String>)> {
    let url = crate::model::endpoint(&config.base_url)
        .map_err(|msg| AppError::invalid("memory_config", msg))?;
    let token = service_token(storage, "memory", crate::model::is_local_or_lan(&url))?;
    Ok((url, token))
}

fn memory_provider(storage: &Storage, config: &MemoryConfig) -> CommandResult<OpenVikingProvider> {
    let (url, token) = memory_access(storage, config)?;
    Ok(OpenVikingProvider::new(
        url,
        config.namespace.clone(),
        token,
    ))
}

fn enabled_memory_provider(storage: &Storage) -> CommandResult<Option<OpenVikingProvider>> {
    memory_config(storage)?
        .filter(|c| c.enabled)
        .map(|config| memory_provider(storage, &config))
        .transpose()
}

pub(crate) fn queue_saved_note(storage: &mut Storage, note_id: &str, prefs: &AppSettings) -> bool {
    let organize =
        prefs.auto_organize && storage.model_config().ok().flatten().is_some_and(|c| c.2);
    let memory = if prefs.auto_sync_memory {
        memory_config(storage)
            .ok()
            .flatten()
            .filter(|c| c.enabled && (c.sync_resources || c.commit_memories))
    } else {
        None
    };
    storage
        .enqueue_capture_jobs(note_id, organize, memory.as_ref())
        .unwrap_or_else(|error| {
            eprintln!("保存后后台任务未入队（{}）", error.safe_message);
            false
        })
}

fn process_background_job(app: &AppHandle, backfill: bool) -> CommandResult<bool> {
    let Some(workspace) = active_workspace(app)? else {
        return Ok(false);
    };
    // Hold no app-state lock during network I/O or while settling the original workspace's job.
    workspace.with_storage(|storage| {
        if backfill
            && settings::settings_path(app)
                .and_then(|path| settings::read(&path))
                .unwrap_or_default()
                .auto_sync_memory
            && let Ok(Some(config)) = memory_config(storage)
            && let Err(error) = storage.enqueue_memory_sync(&config, None, true)
        {
            eprintln!("后台补推排队失败（{}）", error.safe_message);
        }
        let emit_note = |note_id: &str| {
            let _ = app.emit_to("main", "note-updated", json!({ "noteId": note_id }));
        };
        let Some((note_id, organized)) = run_background_job(storage, emit_note)? else {
            return Ok(false);
        };
        emit_note(&note_id);
        if organized {
            let _ = app.emit_to("main", "drawer-badge-updated", ());
        }
        Ok(true)
    })
}

fn run_background_job(
    storage: &mut Storage,
    started_organizing: impl FnOnce(&str),
) -> CommandResult<Option<(String, bool)>> {
    let config = memory_config(storage).ok().flatten().filter(|c| c.enabled);
    let Some(job) = storage.next_background_job(config.as_ref())? else {
        return Ok(None);
    };
    if job.kind == "organize" {
        started_organizing(&job.note_id);
        match storage.prepare_organize_job(&job) {
            Ok(mut input) => {
                let result = (|| {
                    let url = crate::model::chat_endpoint(&input.model.0)?;
                    let token = model_token(storage, crate::model::is_local_or_lan(&url))
                        .map_err(|e| e.safe_message)?;
                    if let Some(provider) = config
                        .as_ref()
                        .and_then(|c| memory_provider(storage, c).ok())
                    {
                        let query: String = input
                            .lines
                            .iter()
                            .map(|(_, line)| line.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                            .chars()
                            .take(1000)
                            .collect();
                        if let Ok(hits) = provider.search(&query, 3, None, None) {
                            let own_uri = storage
                                .resource_batch(&job.path)
                                .ok()
                                .map(|(_, relative_uri)| provider.resource_uri(&relative_uri));
                            input.related.extend(
                                hits.into_iter()
                                    .filter(|hit| own_uri.as_deref() != Some(hit.uri.as_str()))
                                    .map(|hit| {
                                        json!({
                                            "kind": hit.kind,
                                            "provider": provider.provider_id(),
                                            "path": hit.uri,
                                            "title": hit.title,
                                            "capturedAt": hit.updated_at,
                                            "excerpt": hit.snippet,
                                        })
                                    }),
                            );
                        }
                    }
                    crate::model::generate(
                        &url,
                        &input.model.1,
                        token.as_deref(),
                        ORGANIZE_PROMPT,
                        json!({
                            "lines": input.lines,
                            "relatedNotes": input.related,
                            "wikiTargets": input.wiki_targets,
                        }),
                    )
                })();
                storage.complete_organize_job(&input, result)?;
            }
            Err(message) => storage.fail_organize_job(&job.id, &job.note_id, &message)?,
        }
        return Ok(Some((job.note_id, true)));
    }

    let provider = config
        .as_ref()
        .ok_or_else(|| "OpenViking 尚未启用".to_owned())
        .and_then(|c| memory_provider(storage, c).map_err(|e| e.safe_message));
    let tags = ["app=flash-marking-down".to_owned()];
    if job.kind == "ov_write_resource" {
        let (through_row, relative_uri) = storage.resource_batch(&job.path)?;
        let markdown = storage.read_note_markdown(&job.path);
        let result = provider.and_then(|ov| {
            let markdown = markdown.as_ref().map_err(String::clone)?;
            ov.upsert_daily_note(&relative_uri, markdown, &tags)
                .map_err(|e| e.safe_message)
        });
        storage.complete_resource_job(&job, through_row, markdown.as_deref().ok(), result)?;
    } else {
        let result = provider.and_then(|ov| {
            let (title, markdown) = storage.memory_commit_input(&job)?;
            ov.commit_note_memory(&title, &markdown, &tags)
                .map_err(|e| e.safe_message)
        });
        storage.complete_memory_job(&job, result)?;
    }
    Ok(Some((job.note_id, false)))
}

fn model_dto(storage: &Storage, config: ModelConfig) -> CommandResult<ModelConfigDto> {
    let requires_credential = crate::model::chat_endpoint(&config.0)
        .map(|url| !crate::model::is_local_or_lan(&url))
        .unwrap_or(true);
    let cred_state = model_credential_state(storage)?;
    let ready =
        config.2 && (cred_state == "linked" || (!requires_credential && cred_state == "unlinked"));
    Ok(ModelConfigDto {
        endpoint: config.0,
        model: config.1,
        enabled: config.2,
        ready,
        requires_credential,
    })
}

#[tauri::command]
pub(crate) async fn get_model_config(
    window: WebviewWindow,
    session: String,
) -> CommandResult<Option<ModelConfigDto>> {
    with_workspace(window, session, |storage| {
        let Some(config) = storage
            .model_config()
            .map_err(|msg| m2_err("model_config", msg))?
        else {
            return Ok(None);
        };
        model_dto(storage, config).map(Some)
    })
    .await
}

#[derive(serde::Serialize)]
pub(crate) struct IntegrationsConfig {
    model: Option<ModelConfigDto>,
    memory: Option<crate::model::MemoryConfig>,
    credentials: [crate::dto::CredentialStatusDto; 2],
}

#[tauri::command]
pub(crate) async fn get_integrations_config(
    window: WebviewWindow,
    session: String,
) -> CommandResult<IntegrationsConfig> {
    with_workspace(window, session, |storage| {
        let model = storage
            .model_config()
            .map_err(|msg| m2_err("model_config", msg))?
            .map(|config| model_dto(storage, config))
            .transpose()?;
        Ok(IntegrationsConfig {
            model,
            memory: memory_config(storage)?,
            credentials: [
                crate::credential_commands::status(storage, "model")?,
                crate::credential_commands::status(storage, "memory")?,
            ],
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn save_model_config(
    window: WebviewWindow,
    session: String,
    endpoint: String,
    model: String,
    enabled: bool,
    secret: Option<String>,
) -> CommandResult<()> {
    let config = crate::model::ModelSettings::new(&endpoint, &model, enabled)
        .map_err(|msg| AppError::invalid("model_config", msg))?;
    with_workspace(window, session, move |storage| {
        storage.save_service_config("model", &config, secret.as_deref().unwrap_or(""))
    })
    .await
}

#[tauri::command]
pub(crate) async fn save_memory_config(
    window: WebviewWindow,
    session: String,
    base_url: String,
    namespace: String,
    enabled: bool,
    sync_resources: Option<bool>,
    commit_memories: Option<bool>,
    secret: Option<String>,
) -> CommandResult<()> {
    let config = MemoryConfig::new(
        &base_url,
        &namespace,
        enabled,
        sync_resources.unwrap_or(true),
        commit_memories.unwrap_or(true),
    )
    .map_err(|msg| AppError::invalid("memory_config", msg))?;
    let app = window.app_handle().clone();
    with_workspace(window, session, move |storage| {
        storage.save_service_config("memory", &config, secret.as_deref().unwrap_or(""))
    })
    .await?;
    if enabled {
        wake_memory_backfill(&app);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn test_model_connection(
    window: WebviewWindow,
    session: String,
) -> CommandResult<String> {
    let (url, model, token) = with_workspace(window, session, |storage| {
        let (endpoint, model, _) = storage
            .model_config()
            .map_err(|msg| m2_err("model_config", msg))?
            .ok_or_else(|| AppError::invalid("model_config", "请先保存模型配置"))?;
        let url = crate::model::chat_endpoint(&endpoint)
            .map_err(|msg| AppError::invalid("model_config", msg))?;
        let token = model_token(storage, crate::model::is_local_or_lan(&url))?;
        Ok((url, model, token))
    })
    .await?;
    async_runtime::spawn_blocking(move || crate::model::probe_model(&url, &model, token.as_deref()))
        .await
        .map_err(|_| m2_err("connection", "连接测试已中断".into()))?
}

#[tauri::command]
pub(crate) async fn list_remote_models(
    window: WebviewWindow,
    session: String,
    endpoint: String,
    secret: Option<String>,
) -> CommandResult<Vec<String>> {
    let url = crate::model::models_endpoint(&endpoint)
        .map_err(|msg| AppError::invalid("model_config", msg))?;
    let explicit = secret
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let token = if let Some(value) = explicit {
        Some(value)
    } else {
        let local = crate::model::is_local_or_lan(&url);
        with_workspace(window, session, move |storage| {
            match model_token(storage, local) {
                Ok(token) => Ok(token),
                Err(_) if local => Ok(None),
                Err(err) => Err(err),
            }
        })
        .await?
    };
    async_runtime::spawn_blocking(move || crate::model::list_models(&url, token.as_deref()))
        .await
        .map_err(|_| m2_err("model_config", "获取模型列表已中断".into()))?
}

#[tauri::command]
pub(crate) async fn test_memory_connection(
    window: WebviewWindow,
    session: String,
) -> CommandResult<String> {
    let (url, token) = with_workspace(window, session, |storage| {
        let config = memory_config(storage)?
            .ok_or_else(|| AppError::invalid("memory_config", "请先保存 OpenViking 配置"))?;
        memory_access(storage, &config)
    })
    .await?;
    async_runtime::spawn_blocking(move || crate::model::probe_memory(&url, token.as_deref()))
        .await
        .map_err(|_| m2_err("connection", "连接测试已中断".into()))?
}

#[tauri::command]
pub(crate) async fn list_notes(
    window: WebviewWindow,
    session: String,
) -> CommandResult<Vec<NoteSummaryDto>> {
    with_workspace(window, session, |storage| {
        storage.list_notes().map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn get_note_detail(
    window: WebviewWindow,
    session: String,
    id_or_path: String,
) -> CommandResult<NoteDetailDto> {
    with_workspace(window, session, move |storage| {
        storage.get_note_detail(&id_or_path).map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn update_note(
    window: WebviewWindow,
    session: String,
    id_or_path: String,
    expected_hash: String,
    body: String,
    reorganize: Option<bool>,
) -> CommandResult<NoteDetailDto> {
    let app = window.app_handle().clone();
    let prefs = settings::settings_path(&app)
        .and_then(|path| settings::read(&path))
        .unwrap_or_default();
    let (detail, wake) = with_workspace(window, session, move |storage| {
        update_note_and_queue(
            storage,
            &id_or_path,
            &expected_hash,
            &body,
            reorganize.unwrap_or(true),
            &prefs,
        )
    })
    .await?;
    let _ = app.emit_to("main", "note-updated", json!({ "noteId": detail.id }));
    if wake {
        wake_organize_worker(&app);
    }
    Ok(detail)
}

fn update_note_and_queue(
    storage: &mut Storage,
    id_or_path: &str,
    expected_hash: &str,
    body: &str,
    reorganize: bool,
    prefs: &AppSettings,
) -> CommandResult<(NoteDetailDto, bool)> {
    let (locator, note_id) = storage.update_note(id_or_path, expected_hash, body)?;
    let note_id = match note_id {
        None if reorganize => Some(storage.ensure_note_for_source(&locator)?),
        id => id,
    };
    let wake = if let Some(id) = note_id {
        let organize = reorganize
            && storage
                .model_config()
                .map_err(|message| m2_err("organize", message))?
                .is_some_and(|config| config.2);
        let memory = if prefs.auto_sync_memory {
            memory_config(storage).ok().flatten().filter(|config| {
                config.enabled && (config.sync_resources || config.commit_memories)
            })
        } else {
            None
        };
        // A running job may already be working from the previous version. Keep one follow-up.
        storage.enqueue_note_jobs(&id, organize, memory.as_ref(), true)? || organize
    } else {
        false
    };
    Ok((storage.get_note_detail(&locator)?, wake))
}

#[tauri::command]
pub(crate) async fn trigger_note_organize(
    window: WebviewWindow,
    session: String,
    id_or_path: String,
) -> CommandResult<NoteDetailDto> {
    let app = window.app_handle().clone();
    let detail = with_workspace(window, session, move |storage| {
        let enabled = storage
            .model_config()
            .map_err(|msg| m2_err("organize", msg))?
            .is_some_and(|c| c.2);
        if !enabled {
            return Err(AppError::invalid("organize", "请先在设置中配置并启用模型"));
        }
        let note_id = storage.ensure_note_for_source(&id_or_path)?;
        storage.enqueue_organize_job(&note_id)?;
        storage.get_note_detail(&note_id).map_err(AppError::from)
    })
    .await?;
    wake_organize_worker(&app);
    Ok(detail)
}

#[tauri::command]
pub(crate) async fn retry_memory_sync(
    window: WebviewWindow,
    session: String,
    note_id: Option<String>,
) -> CommandResult<usize> {
    let app = window.app_handle().clone();
    let count = with_workspace(window, session, move |storage| {
        queue_memory_retry(storage, note_id.as_deref())
    })
    .await?;
    wake_organize_worker(&app);
    Ok(count)
}

fn queue_memory_retry(storage: &mut Storage, note_id: Option<&str>) -> CommandResult<usize> {
    let config = memory_config(storage)?
        .filter(|c| c.enabled)
        .ok_or_else(|| AppError::invalid("memory_sync", "请先在设置中启用 OpenViking"))?;
    if !(config.sync_resources || config.commit_memories) {
        return Ok(0);
    }
    let note = note_id
        .map(|id| storage.ensure_note_for_source(id))
        .transpose()?;
    storage
        .enqueue_memory_sync(&config, note.as_deref(), false)
        .map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn drawer_counts(
    window: WebviewWindow,
    session: String,
) -> CommandResult<DrawerCountsDto> {
    with_workspace(window, session, |storage| {
        storage.drawer_counts().map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn list_conflicts(
    window: WebviewWindow,
    session: String,
) -> CommandResult<Vec<ConflictDto>> {
    with_workspace(window, session, |storage| {
        storage.list_conflicts().map_err(AppError::from)
    })
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_conflict(
    window: WebviewWindow,
    session: String,
    topic: String,
    current_label: Option<String>,
    current_path: Option<String>,
    current_text: String,
    incoming_label: Option<String>,
    incoming_path: Option<String>,
    incoming_text: String,
) -> CommandResult<ConflictDto> {
    let app = window.app_handle().clone();
    let created = with_workspace(window, session, move |storage| {
        storage
            .create_conflict(
                &topic,
                current_label.as_deref(),
                current_path.as_deref(),
                &current_text,
                incoming_label.as_deref(),
                incoming_path.as_deref(),
                &incoming_text,
            )
            .map_err(|msg| AppError::invalid("conflict", msg))
    })
    .await?;
    let _ = app.emit_to("main", "drawer-badge-updated", ());
    Ok(created)
}

#[tauri::command]
pub(crate) async fn resolve_conflict(
    window: WebviewWindow,
    session: String,
    id: String,
    status: String,
    resolution_note: Option<String>,
    wiki_target: Option<String>,
) -> CommandResult<ConflictDto> {
    let app = window.app_handle().clone();
    let updated = with_workspace(window, session, move |storage| {
        storage
            .resolve_conflict(
                &id,
                &status,
                resolution_note.as_deref(),
                wiki_target.as_deref(),
            )
            .map_err(AppError::from)
    })
    .await?;
    let _ = app.emit_to("main", "drawer-badge-updated", ());
    Ok(updated)
}

#[tauri::command]
pub(crate) async fn list_wiki_documents(
    window: WebviewWindow,
    session: String,
) -> CommandResult<Vec<WikiDocumentDto>> {
    let workspace =
        with_workspace(window, session, |storage| Ok(storage.workspace_handle())).await?;
    // Reading every Markdown file must not hold the quick-capture workspace lock.
    async_runtime::spawn_blocking(move || {
        workspace.with_storage(|storage| storage.list_wiki_documents().map_err(AppError::from))
    })
    .await
    .map_err(|_| {
        AppError::from_message("CANCELLED", "wiki", true, "读取 Wiki 文档中断，请刷新重试")
    })?
}

#[tauri::command]
pub(crate) async fn wiki_targets(
    window: WebviewWindow,
    session: String,
) -> CommandResult<Vec<WikiTargetDto>> {
    with_workspace(window, session, |storage| {
        Ok(storage
            .wiki_targets()?
            .into_iter()
            .map(|(kind, path)| WikiTargetDto { kind, path })
            .collect())
    })
    .await
}

#[tauri::command]
pub(crate) async fn map_wiki_target(
    window: WebviewWindow,
    session: String,
    kind: String,
    path: String,
) -> CommandResult<()> {
    with_workspace(window, session, move |storage| {
        storage
            .map_wiki_target(&kind, &path)
            .map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn unmap_wiki_target(
    window: WebviewWindow,
    session: String,
    path: String,
) -> CommandResult<()> {
    with_workspace(window, session, move |storage| {
        storage.unmap_wiki_target(&path).map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn create_wiki_target(
    window: WebviewWindow,
    session: String,
    path: String,
) -> CommandResult<()> {
    with_workspace(window, session, move |storage| {
        storage.create_wiki_target(&path).map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn list_wiki_suggestions(
    window: WebviewWindow,
    session: String,
) -> CommandResult<Vec<WikiSuggestionDto>> {
    with_workspace(window, session, |storage| {
        storage.list_wiki_suggestions().map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn create_wiki_suggestion(
    window: WebviewWindow,
    session: String,
    note_id: Option<String>,
    target_path: String,
    content: String,
    reason: String,
) -> CommandResult<WikiSuggestionDto> {
    let app = window.app_handle().clone();
    let item = with_workspace(window, session, move |storage| {
        storage
            .create_wiki_suggestion(note_id.as_deref(), &target_path, &content, &reason)
            .map_err(AppError::from)
    })
    .await?;
    let _ = app.emit_to("main", "drawer-badge-updated", ());
    Ok(item)
}

#[tauri::command]
pub(crate) async fn apply_wiki_suggestion(
    window: WebviewWindow,
    session: String,
    id: String,
) -> CommandResult<WikiSuggestionDto> {
    let app = window.app_handle().clone();
    let updated = with_workspace(window, session, move |storage| {
        storage.apply_wiki_suggestion(&id)?;
        storage
            .list_wiki_suggestions()?
            .into_iter()
            .find(|item| item.id == id)
            .ok_or_else(|| AppError::invalid("wiki", "找不到更新后的 Wiki 建议"))
    })
    .await?;
    let _ = app.emit_to("main", "drawer-badge-updated", ());
    Ok(updated)
}

#[tauri::command]
pub(crate) async fn recompare_wiki_suggestion(
    window: WebviewWindow,
    session: String,
    id: String,
) -> CommandResult<WikiSuggestionDto> {
    let app = window.app_handle().clone();
    let updated = with_workspace(window, session, move |storage| {
        storage
            .recompare_wiki_suggestion(&id)
            .map_err(AppError::from)
    })
    .await?;
    let _ = app.emit_to("main", "drawer-badge-updated", ());
    Ok(updated)
}

#[tauri::command]
pub(crate) async fn reject_wiki_suggestion(
    window: WebviewWindow,
    session: String,
    id: String,
) -> CommandResult<()> {
    let app = window.app_handle().clone();
    with_workspace(window, session, move |storage| {
        storage.reject_wiki_suggestion(&id).map_err(AppError::from)
    })
    .await?;
    let _ = app.emit_to("main", "drawer-badge-updated", ());
    Ok(())
}

#[tauri::command]
pub(crate) async fn undo_wiki_suggestion(
    window: WebviewWindow,
    session: String,
    id: String,
) -> CommandResult<WikiSuggestionDto> {
    let app = window.app_handle().clone();
    let updated = with_workspace(window, session, move |storage| {
        storage.undo_wiki_suggestion(&id).map_err(AppError::from)
    })
    .await?;
    let _ = app.emit_to("main", "drawer-badge-updated", ());
    Ok(updated)
}

#[tauri::command]
pub(crate) async fn list_tasks(
    window: WebviewWindow,
    session: String,
) -> CommandResult<TaskListDto> {
    with_workspace(window, session, |storage| {
        let (path, revision, entries, repair_messages) = storage.list_all_tasks()?;
        Ok(TaskListDto {
            path,
            revision,
            entries,
            repair_messages,
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn toggle_task(
    window: WebviewWindow,
    session: String,
    path: String,
    task_id: String,
    expected_line: String,
    done: bool,
) -> CommandResult<()> {
    let app = window.app_handle().clone();
    let wrote_markdown = with_workspace(window, session, move |storage| {
        Ok(storage.toggle_task(&path, &task_id, &expected_line, done)?)
    })
    .await?;
    if wrote_markdown {
        wake_memory_backfill(&app);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn list_conversations(
    window: WebviewWindow,
    session: String,
) -> CommandResult<Vec<ConversationSummaryDto>> {
    with_workspace(window, session, |storage| {
        storage.list_conversations().map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn get_conversation(
    window: WebviewWindow,
    session: String,
    id: String,
) -> CommandResult<ConversationDetailDto> {
    with_workspace(window, session, move |storage| {
        storage.get_conversation(&id).map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn delete_conversation(
    window: WebviewWindow,
    session: String,
    id: String,
) -> CommandResult<()> {
    with_workspace(window, session, move |storage| {
        storage.delete_conversation(&id).map_err(AppError::from)
    })
    .await
}

fn recall_openviking(
    provider: CommandResult<Option<OpenVikingProvider>>,
    question: &str,
) -> OpenVikingRecall {
    let Some(provider) = provider? else {
        return Ok(None);
    };
    let hits = provider.search(question, 6, None, None)?;
    Ok(Some((provider.resource_uri("daily/"), hits)))
}

#[tauri::command]
pub(crate) async fn send_chat_message(
    window: WebviewWindow,
    session: String,
    conversation_id: Option<String>,
    message: String,
) -> CommandResult<ChatTurnDto> {
    let app = window.app_handle().clone();
    let question = validate_chat_question(&message)?.to_owned();
    let local_question = question.clone();
    let (model, token, mut prepared, recall) =
        with_workspace(window.clone(), session.clone(), move |storage| {
            let config = storage
                .model_config()
                .map_err(|msg| m2_err("chat", msg))?
                .filter(|c| c.2)
                .ok_or_else(|| AppError::invalid("chat", "请先配置并启用模型后再发起记忆对话"))?;
            let url = crate::model::chat_endpoint(&config.0)
                .map_err(|msg| AppError::invalid("chat", msg))?;
            let token = model_token(storage, crate::model::is_local_or_lan(&url))?;
            let provider = enabled_memory_provider(storage);
            let ov_question = local_question.clone();
            let recall =
                async_runtime::spawn_blocking(move || recall_openviking(provider, &ov_question));
            let prepared =
                storage.prepare_chat_turn(conversation_id.as_deref(), &local_question)?;
            Ok(((url, config.1), token, prepared, recall))
        })
        .await?;

    prepared.add_openviking_recall(
        recall
            .await
            .unwrap_or_else(|_| Err(m2_err("chat", "OpenViking 检索中断".into()))),
    );
    let conv_id = prepared.conversation_id.clone();
    let stream_app = app.clone();
    let stream_conv_id = conv_id.clone();
    let messages = prepared.model_messages(&question);
    let assistant_text = async_runtime::spawn_blocking(move || {
        default_http_transport().stream_chat(
            &model.0,
            &model.1,
            token.as_deref(),
            &messages,
            &mut |delta| {
                let _ = stream_app.emit_to(
                    "main",
                    "chat-chunk",
                    json!({ "conversationId": stream_conv_id, "delta": delta }),
                );
            },
        )
    })
    .await
    .map_err(|_| m2_err("chat", "对话请求被中断".into()))??;

    let turn = with_workspace(window, session, move |storage| {
        storage
            .commit_chat_turn(&prepared, &message, &assistant_text)
            .map_err(AppError::from)
    })
    .await?;

    let _ = app.emit_to("main", "chat-done", json!({ "conversationId": conv_id }));
    Ok(turn)
}

#[tauri::command]
pub(crate) async fn read_memory_item(
    window: WebviewWindow,
    session: String,
    uri: String,
) -> CommandResult<MemoryItemDto> {
    let provider = with_workspace(window, session, |storage| {
        enabled_memory_provider(storage)?
            .ok_or_else(|| AppError::invalid("memory_read", "请先在设置中启用 OpenViking"))
    })
    .await?;
    async_runtime::spawn_blocking(move || {
        let content = provider.read_content(&uri)?;
        let title = memory_item_title(&uri, markdown_body(&content), "");
        let kind = if !uri.starts_with("viking://resources/") && uri.contains("/memories/") {
            "ov_memory"
        } else {
            "ov_resource"
        };
        Ok(MemoryItemDto {
            uri,
            kind: kind.into(),
            title,
            content,
        })
    })
    .await
    .map_err(|_| m2_err("memory_read", "OpenViking 原文读取中断".into()))?
}

#[cfg(test)]
mod r3_tests;
