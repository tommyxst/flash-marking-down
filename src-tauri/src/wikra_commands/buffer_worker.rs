use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Instant;

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::capture_commands::active_workspace;
use crate::dto::AppError;
use crate::dto::wikra::ChangesetItemDto;
use crate::settings;
use crate::storage::WorkspaceStorage;
use crate::storage::wikra::organize::ModelAccess;

enum Message {
    Wake(WorkspaceStorage),
    Stop,
}

struct BufferWorker(Sender<Message>);

pub(crate) fn init_buffer_worker(app: &AppHandle) -> std::io::Result<()> {
    let (sender, receiver) = mpsc::channel();
    let handle = app.clone();
    std::thread::Builder::new()
        .name("wikra-buffer".to_owned())
        .spawn(move || {
            run(receiver, |workspace| {
                match process_due(&handle, workspace) {
                    Ok(()) => true,
                    Err(err) => {
                        eprintln!("Wikra 缓冲队列处理失败（{}）", err.safe_message);
                        emit_to_active(
                            &handle,
                            workspace,
                            "wikra-organize-finished",
                            json!({ "summary": err.safe_message, "error": err }),
                        );
                        false
                    }
                }
            });
        })?;
    app.manage(BufferWorker(sender));
    Ok(())
}

pub(crate) fn stop_buffer_worker(app: &AppHandle) {
    if let Some(worker) = app.try_state::<BufferWorker>() {
        let _ = worker.0.send(Message::Stop);
    }
}

fn track(app: &AppHandle, workspace: WorkspaceStorage) {
    if let Some(worker) = app.try_state::<BufferWorker>() {
        let _ = worker.0.send(Message::Wake(workspace));
    }
}

pub(crate) fn wake_active_buffer(app: &AppHandle) {
    if let Ok(Some(workspace)) = active_workspace(app) {
        track(app, workspace);
    }
}

pub(crate) fn wake_buffer(app: &AppHandle, workspace: WorkspaceStorage, seconds: u32) {
    emit_to_active(
        app,
        &workspace,
        "wikra-buffer-updated",
        json!({ "bufferSecondsLeft": seconds }),
    );
    track(app, workspace);
}

fn emit_to_active(
    app: &AppHandle,
    workspace: &WorkspaceStorage,
    event: &str,
    payload: serde_json::Value,
) {
    if active_workspace(app).is_ok_and(|active| {
        active.is_some_and(|active| active.database_path() == workspace.database_path())
    }) {
        let _ = app.emit_to("main", event, payload);
    }
}

fn run(receiver: Receiver<Message>, mut on_due: impl FnMut(&WorkspaceStorage) -> bool) {
    let mut schedule: HashMap<PathBuf, (WorkspaceStorage, Instant)> = HashMap::new();
    loop {
        // Consume pending wakeups before inspecting the deadline in case it just changed.
        match receiver.try_recv() {
            Ok(Message::Wake(workspace)) => {
                refresh_schedule(&mut schedule, workspace);
                continue;
            }
            Ok(Message::Stop) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => (),
        }
        let next = schedule.values().min_by_key(|(_, deadline)| *deadline);
        let message = if let Some((workspace, deadline)) = next {
            let delay = deadline.saturating_duration_since(Instant::now());
            if delay.is_zero() {
                let workspace = workspace.clone();
                if on_due(&workspace) {
                    refresh_schedule(&mut schedule, workspace);
                } else {
                    schedule.remove(workspace.database_path());
                }
                continue;
            }
            match receiver.recv_timeout(delay) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else if let Ok(message) = receiver.recv() {
            message
        } else {
            break;
        };
        match message {
            Message::Wake(workspace) => refresh_schedule(&mut schedule, workspace),
            Message::Stop => break,
        }
    }
}

fn refresh_schedule(
    schedule: &mut HashMap<PathBuf, (WorkspaceStorage, Instant)>,
    workspace: WorkspaceStorage,
) {
    let key = workspace.database_path().to_owned();
    schedule.remove(&key);
    match workspace.with_storage(|storage| storage.next_buffer_delay()) {
        Ok(Some(delay)) => {
            if let Some(deadline) = Instant::now().checked_add(delay) {
                schedule.insert(key, (workspace, deadline));
            }
        }
        Ok(None) => (),
        Err(err) => eprintln!("Wikra 缓冲倒计时不可用（{}）", err.safe_message),
    }
}

fn process_due(app: &AppHandle, workspace: &WorkspaceStorage) -> Result<(), AppError> {
    let ids = workspace.with_storage(|storage| storage.due_buffer_items())?;
    if ids.is_empty() {
        return Ok(());
    }
    emit_to_active(
        app,
        workspace,
        "wikra-buffer-updated",
        json!({ "bufferSecondsLeft": 0 }),
    );
    let preferences = settings::settings_path(app).and_then(|path| settings::read(&path));
    let result = preferences
        .as_ref()
        .map_err(|_| {
            AppError::from_message(
                "SETTINGS_FAILED",
                "wikra_organize",
                true,
                "无法读取模型发送授权，请检查设置后重新整理",
            )
        })
        .and_then(|preferences| {
            compile_batch(workspace, &ids, preferences.allow_note_fulltext_to_model)
        });
    let quiet = preferences
        .as_ref()
        .is_ok_and(|preferences| preferences.quiet_organize);
    let payload = match result {
        Ok(changeset) => json!({
            "changesetId": changeset.id,
            "quietOrganize": quiet,
            "summary": (!quiet).then_some(changeset.summary),
        }),
        Err(err) => {
            workspace.with_storage(|storage| storage.pause_buffer_items(&ids))?;
            json!({ "summary": err.safe_message, "error": err, "quietOrganize": quiet })
        }
    };
    let seconds = workspace.with_storage(|storage| storage.buffer_seconds_left())?;
    emit_to_active(
        app,
        workspace,
        "wikra-buffer-updated",
        json!({ "bufferSecondsLeft": seconds }),
    );
    emit_to_active(app, workspace, "wikra-organize-finished", payload);
    Ok(())
}

fn compile_batch(
    workspace: &WorkspaceStorage,
    ids: &[String],
    allow_fulltext: bool,
) -> Result<ChangesetItemDto, AppError> {
    if !allow_fulltext {
        return Err(AppError::invalid(
            "wikra_organize",
            "随手记全文发送已关闭，可在设置中启用后重新整理",
        ));
    }
    let (batch, model) = workspace.with_storage(|storage| {
        let model = ModelAccess::load(storage)?.ok_or_else(|| {
            AppError::invalid("wikra_organize", "请先在设置中配置并启用模型，再重新整理")
        })?;
        Ok::<_, AppError>((storage.prepare_buffer_batch(ids)?, model))
    })?;
    let output = model.compile(&batch.sources)?;
    workspace
        .with_storage(|storage| storage.apply_buffer_compilation(&batch, &model.alias, output))
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::path::Path;
    use std::time::Duration;

    use crate::model::ModelSettings;
    use crate::storage::Storage;
    use crate::storage::tests::TestDir;
    use crate::storage::wikra::organize::CaptureMode;
    use crate::workspace::STATE_DIR;

    use super::*;

    #[test]
    fn timer_dispatches_due_captures_in_the_original_workspace() {
        let origin = TestDir::new();
        let unrelated = TestDir::new();
        let mut storage = Storage::open(&origin.0, Path::new("raw/notes"), STATE_DIR).unwrap();
        let other = Storage::open(&unrelated.0, Path::new("raw/notes"), STATE_DIR).unwrap();
        let record = storage
            .save_record_with_buffer(
                "timer",
                "draft",
                1,
                "一秒后整理",
                Some(CaptureMode::Queue(1)),
            )
            .unwrap();
        assert!(storage.due_buffer_items().unwrap().is_empty());
        let workspace = storage.into_workspace();
        let (wake, receiver) = mpsc::channel();
        let (results, dispatched) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            run(receiver, |workspace| {
                let ids = workspace
                    .with_storage(|storage| storage.due_buffer_items())
                    .unwrap();
                let result = workspace.with_storage(|storage| storage.pause_buffer_items(&ids));
                results
                    .send((workspace.database_path().to_owned(), ids, result))
                    .unwrap();
                true
            })
        });
        wake.send(Message::Wake(workspace.clone()))
            .map_err(|_| ())
            .unwrap();
        wake.send(Message::Wake(other.into_workspace()))
            .map_err(|_| ())
            .unwrap();
        let result = dispatched.recv_timeout(Duration::from_secs(8));
        wake.send(Message::Stop).map_err(|_| ()).unwrap();
        worker.join().unwrap();
        let (path, ids, paused) = result.unwrap();
        paused.unwrap();
        assert_eq!(path, workspace.database_path());
        assert_eq!(ids, [record.record_id]);
    }

    #[test]
    fn disabled_note_fulltext_does_not_contact_configured_model() {
        let dir = TestDir::new();
        let mut storage = Storage::open(&dir.0, Path::new("raw/notes"), STATE_DIR).unwrap();
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        server.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", server.local_addr().unwrap());
        let settings = ModelSettings::new(&endpoint, "test-model", true).unwrap();
        storage.save_service_config("model", &settings, "").unwrap();
        let record = storage
            .save_record_with_buffer(
                "privacy",
                "draft",
                1,
                "正文只保留在本地",
                Some(CaptureMode::Now),
            )
            .unwrap();
        let workspace = storage.into_workspace();
        let err = compile_batch(&workspace, &[record.record_id], false)
            .err()
            .unwrap();
        assert!(err.safe_message.contains("全文发送已关闭"));
        assert_eq!(
            server.accept().err().unwrap().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
