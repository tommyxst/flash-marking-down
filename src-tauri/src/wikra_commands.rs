use tauri::{Manager, WebviewWindow, async_runtime};

use crate::capture_commands::{with_workspace, workspace_for_session};
use crate::dto::AppError;
use crate::dto::wikra::*;
use crate::storage::wikra::inbox::{CompileMode, prepare_clip};
use crate::storage::wikra::organize::{ModelAccess, generate_teachback, local_teachback};
use crate::storage::{Storage, StorageError};

mod buffer_worker;

pub(crate) use buffer_worker::{
    init_buffer_worker, stop_buffer_worker, wake_active_buffer, wake_buffer,
};

type CommandResult<T> = Result<T, AppError>;

const STAGE: &str = "wikra";

async fn blocking<T: Send + 'static>(
    action: impl FnOnce() -> CommandResult<T> + Send + 'static,
) -> CommandResult<T> {
    async_runtime::spawn_blocking(action).await.map_err(|_| {
        AppError::from_message("CANCELLED", STAGE, true, "知识库操作中断，请刷新后重试")
    })?
}

async fn read_workspace<T: Send + 'static>(
    window: WebviewWindow,
    session: String,
    action: impl FnOnce(&mut Storage) -> CommandResult<T> + Send + 'static,
) -> CommandResult<T> {
    let workspace = workspace_for_session(window.clone(), session.clone()).await?;
    let result = blocking(move || workspace.with_storage(action)).await;
    workspace_for_session(window, session).await?;
    result
}

#[tauri::command]
pub(crate) async fn list_library_tree(
    window: WebviewWindow,
    session: String,
) -> CommandResult<LibraryTreeDto> {
    read_workspace(window, session, |storage| Ok(storage.list_library_tree()?)).await
}

#[tauri::command]
pub(crate) async fn read_document_detail(
    window: WebviewWindow,
    session: String,
    relative_path: String,
) -> CommandResult<DocumentDetailDto> {
    read_workspace(window, session, move |storage| {
        Ok(storage.read_document_detail(&relative_path)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn save_markdown_document(
    window: WebviewWindow,
    session: String,
    relative_path: String,
    expected_hash: String,
    body: String,
) -> CommandResult<DocumentDetailDto> {
    let key = with_workspace(window.clone(), session.clone(), move |storage| {
        Ok(storage.save_markdown_document(&relative_path, &expected_hash, &body)?)
    })
    .await?;
    read_document_detail(window, session, key).await
}

#[tauri::command]
pub(crate) async fn create_markdown_document(
    window: WebviewWindow,
    session: String,
    relative_path: String,
    initial_body: String,
) -> CommandResult<DocumentDetailDto> {
    let key = with_workspace(window.clone(), session.clone(), move |storage| {
        Ok(storage.create_markdown_document(&relative_path, &initial_body)?)
    })
    .await?;
    read_document_detail(window, session, key).await
}

#[tauri::command]
pub(crate) async fn append_human_understanding(
    window: WebviewWindow,
    session: String,
    relative_path: String,
    insight_text: String,
) -> CommandResult<DocumentDetailDto> {
    let key = with_workspace(window.clone(), session.clone(), move |storage| {
        Ok(storage.append_human_understanding(&relative_path, &insight_text)?)
    })
    .await?;
    read_document_detail(window, session, key).await
}

#[tauri::command]
pub(crate) async fn list_inbox_items(
    window: WebviewWindow,
    session: String,
) -> CommandResult<Vec<InboxItemSummaryDto>> {
    read_workspace(window, session, |storage| Ok(storage.list_inbox_items()?)).await
}

#[tauri::command]
pub(crate) async fn get_inbox_item(
    window: WebviewWindow,
    session: String,
    id: String,
) -> CommandResult<InboxItemDetailDto> {
    with_workspace(window, session, move |storage| {
        Ok(storage.open_inbox_item(&id)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn ingest_inbox_input(
    window: WebviewWindow,
    session: String,
    input: String,
    title_hint: Option<String>,
) -> CommandResult<InboxItemDetailDto> {
    workspace_for_session(window.clone(), session.clone()).await?;
    let clip = blocking(move || Ok(prepare_clip(input, title_hint)?)).await?;
    with_workspace(window, session, move |storage| {
        Ok(storage.create_inbox_item(clip)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn complete_inbox_stub(
    window: WebviewWindow,
    session: String,
    id: String,
    pasted_body: String,
) -> CommandResult<InboxItemDetailDto> {
    with_workspace(window, session, move |storage| {
        Ok(storage.complete_inbox_stub(&id, &pasted_body)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn toggle_inbox_paragraph(
    window: WebviewWindow,
    session: String,
    id: String,
    paragraph_id: String,
) -> CommandResult<InboxItemDetailDto> {
    with_workspace(window, session, move |storage| {
        Ok(storage.toggle_inbox_paragraph(&id, &paragraph_id)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn compile_inbox_item(
    window: WebviewWindow,
    session: String,
    id: String,
    mode: String,
) -> CommandResult<InboxItemDetailDto> {
    let mode = CompileMode::parse(&mode)?;
    if mode == CompileMode::Archive {
        return with_workspace(window, session, move |storage| {
            Ok(storage.archive_inbox_item(&id)?)
        })
        .await;
    }
    let item_id = id.clone();
    let prepare = read_workspace(window.clone(), session.clone(), move |storage| {
        let Some(item) = storage.prepare_inbox_compile(&item_id, mode)? else {
            return Ok(None);
        };
        let model = ModelAccess::load(storage)?.ok_or_else(|| {
            AppError::invalid("wikra_inbox", "请先在设置中配置并启用模型，再整理原文")
        })?;
        Ok(Some((item, model)))
    })
    .await?;
    let Some((item, model)) = prepare else {
        return read_workspace(window, session, move |storage| Ok(storage.inbox_item(&id)?)).await;
    };
    let (item, alias, output) = blocking(move || {
        let output = model.compile(&item.sources)?;
        Ok((item, model.alias, output))
    })
    .await?;
    with_workspace(window, session, move |storage| {
        Ok(storage.apply_inbox_compilation(&item, &alias, output)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn list_organize_overview(
    window: WebviewWindow,
    session: String,
) -> CommandResult<OrganizeOverviewDto> {
    read_workspace(window, session, |storage| {
        Ok(storage.list_organize_overview()?)
    })
    .await
}

async fn update_buffer(
    window: WebviewWindow,
    session: String,
    action: impl FnOnce(&mut Storage) -> Result<(), StorageError> + Send + 'static,
) -> CommandResult<OrganizeOverviewDto> {
    let app = window.app_handle().clone();
    let (workspace, seconds) = with_workspace(window.clone(), session.clone(), move |storage| {
        action(storage)?;
        Ok((storage.workspace_handle(), storage.buffer_seconds_left()?))
    })
    .await?;
    wake_buffer(&app, workspace, seconds);
    list_organize_overview(window, session).await
}

#[tauri::command]
pub(crate) async fn cancel_buffer_item(
    window: WebviewWindow,
    session: String,
    item_id: String,
) -> CommandResult<OrganizeOverviewDto> {
    update_buffer(window, session, move |storage| {
        storage.cancel_buffer_item(&item_id)
    })
    .await
}

#[tauri::command]
pub(crate) async fn flush_buffer_now(
    window: WebviewWindow,
    session: String,
) -> CommandResult<OrganizeOverviewDto> {
    update_buffer(window, session, |storage| storage.flush_buffer_now()).await
}

#[tauri::command]
pub(crate) async fn revert_changeset(
    window: WebviewWindow,
    session: String,
    changeset_id: String,
) -> CommandResult<ChangesetItemDto> {
    with_workspace(window, session, move |storage| {
        Ok(storage.revert_changeset(&changeset_id)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn dismiss_suggestion(
    window: WebviewWindow,
    session: String,
    suggestion_id: String,
) -> CommandResult<()> {
    with_workspace(window, session, move |storage| {
        Ok(storage.dismiss_suggestion(&suggestion_id)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn teachback_turn(
    window: WebviewWindow,
    session: String,
    page_path: String,
    step: i64,
    user_answer: Option<String>,
) -> CommandResult<TeachbackTurnDto> {
    if !(1..=3).contains(&step) {
        return Err(AppError::invalid(
            "wikra_teachback",
            "回忆步骤必须为 1、2 或 3",
        ));
    }
    let step = step as u8;
    let (page, model) = read_workspace(window.clone(), session.clone(), move |storage| {
        let page = storage.read_document_detail(&page_path)?;
        if page.kind != "wiki" {
            return Err(AppError::invalid("wikra_teachback", "请选择知识页开始回忆"));
        }
        Ok((page, ModelAccess::load(storage)?))
    })
    .await?;
    let answer = user_answer
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let Some(model) = model else {
        return Ok(local_teachback(&page, step, answer));
    };
    let answer = answer.map(str::to_owned);
    let result = blocking(move || generate_teachback(&model, &page, step, answer.as_deref())).await;
    workspace_for_session(window, session).await?;
    result
}

#[tauri::command]
pub(crate) async fn save_answer_to_wiki(
    window: WebviewWindow,
    session: String,
    conversation_id: String,
    message_id: String,
    target_path: Option<String>,
) -> CommandResult<DocumentDetailDto> {
    let key = with_workspace(window.clone(), session.clone(), move |storage| {
        Ok(storage.save_answer_to_wiki(&conversation_id, &message_id, target_path.as_deref())?)
    })
    .await?;
    read_document_detail(window, session, key).await
}

#[tauri::command]
pub(crate) async fn get_knowledge_graph(
    window: WebviewWindow,
    session: String,
    include_meta_pages: bool,
) -> CommandResult<KnowledgeGraphDto> {
    read_workspace(window, session, move |storage| {
        Ok(storage.get_knowledge_graph(include_meta_pages)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn dismiss_graph_gap(
    window: WebviewWindow,
    session: String,
    gap_id: String,
) -> CommandResult<()> {
    with_workspace(window, session, move |storage| {
        Ok(storage.dismiss_graph_gap(&gap_id)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn global_search(
    window: WebviewWindow,
    session: String,
    query: String,
    limit: Option<i64>,
) -> CommandResult<Vec<GlobalSearchHitDto>> {
    let limit = usize::try_from(limit.unwrap_or(50))
        .map_err(|_| AppError::invalid("wikra_search", "搜索结果条数必须为非负整数"))?;
    read_workspace(window, session, move |storage| {
        Ok(storage.global_search(&query, limit)?)
    })
    .await
}
