use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use tauri::async_runtime::{self, Mutex as AsyncMutex};
use tauri::menu::{Menu, MenuEvent, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{
    AppHandle, Emitter, Manager, WebviewWindow, WebviewWindowBuilder, Window, WindowEvent,
};

use crate::dto::{
    AppError, DraftDto, IndexRebuildDto, OpenedWorkspaceDto, RecentCaptureDto, RecordSourceDto,
    RecoveryErrorDto, SearchHitDto, SearchPageDto,
};
use crate::platform::ForegroundWindow;
use crate::storage::{RecoveryOutcome, SearchReader, Storage, StorageError, error};
use crate::workspace::STATE_DIR;

const SELECTION_FILE: &str = "last-workspace.sqlite";
const CAPTURE_DRAFT: &str = "quick-capture";
const MAIN_WINDOW: &str = "main";
const CAPTURE_WINDOW: &str = "capture";
const TRAY_CAPTURE: &str = "new-note";
const TRAY_WORKBENCH: &str = "open-workbench";
const TRAY_EXIT: &str = "exit-app";

// The event loop must be able to coordinate windows without taking the storage lock.
#[derive(Default)]
pub(crate) struct DesktopWindows {
    capture_active: AtomicBool,
    main_closing: AtomicBool,
    app_exiting: AtomicBool,
    transition: AsyncMutex<()>,
    previous_focus: Mutex<Option<ForegroundWindow>>,
}

impl DesktopWindows {
    fn capture_active(&self) -> bool {
        self.capture_active.load(Ordering::Relaxed)
    }

    fn begin_capture(&self) -> Result<bool, String> {
        if self.app_exiting.load(Ordering::Relaxed) {
            return Err("应用正在退出".to_owned());
        }
        Ok(!self.capture_active.swap(true, Ordering::Relaxed))
    }

    fn capture_closed(&self) {
        self.previous_focus.lock().unwrap().take();
        self.capture_active.store(false, Ordering::Relaxed);
    }

    fn begin_main_close(&self) -> bool {
        self.main_closing.store(true, Ordering::Relaxed);
        true
    }

    fn main_closed(&self) {
        self.main_closing.store(false, Ordering::Relaxed);
    }

    fn begin_exit(&self) {
        self.app_exiting.store(true, Ordering::Relaxed);
    }

    fn allow_window_action(&self) -> Result<(), String> {
        if self.app_exiting.load(Ordering::Relaxed) {
            return Err("应用正在退出".to_owned());
        }
        Ok(())
    }

    fn allow_main_action(&self) -> Result<(), String> {
        self.allow_window_action()?;
        if self.main_closing.load(Ordering::Relaxed) {
            return Err("主窗口正在关闭".to_owned());
        }
        Ok(())
    }

    fn allow_workspace_switch(&self) -> Result<(), String> {
        if self.capture_active() {
            return Err("请先关闭快捷窗再切换工作区".to_owned());
        }
        Ok(())
    }
}

pub(crate) fn on_window_event(window: &Window, event: &WindowEvent) {
    let app = window.app_handle();
    match (window.label(), event) {
        (MAIN_WINDOW, WindowEvent::CloseRequested { api, .. }) => {
            if !app.state::<DesktopWindows>().begin_main_close() {
                api.prevent_close();
            }
        }
        (MAIN_WINDOW, WindowEvent::Destroyed) => app.state::<DesktopWindows>().main_closed(),
        (CAPTURE_WINDOW, WindowEvent::Destroyed) => capture_closed(app),
        _ => {}
    }
}

fn capture_closed(app: &AppHandle) {
    app.state::<DesktopWindows>().capture_closed();
    if let Err(error) = app.emit_to(MAIN_WINDOW, "capture-closed", ()) {
        eprintln!("快捷窗关闭通知失败（{error}）");
    }
}

struct WorkspaceSelection {
    root: String,
    capture_dir: String,
    workspace_id: String,
    file_identity: String,
}

impl WorkspaceSelection {
    fn load(path: &Path) -> Result<Option<Self>, StorageError> {
        match fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(selection_error(error)),
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(selection_error)?;
        connection
            .pragma_update(None, "busy_timeout", 5000)
            .map_err(selection_error)?;
        let initialized: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'last_workspace')",
                [],
                |row| row.get(0),
            )
            .map_err(selection_error)?;
        if !initialized {
            return Ok(None);
        }
        connection
            .query_row(
                "SELECT root, capture_dir, workspace_id, file_identity
                 FROM last_workspace WHERE singleton = 1",
                [],
                |row| {
                    Ok(Self {
                        root: row.get(0)?,
                        capture_dir: row.get(1)?,
                        workspace_id: row.get(2)?,
                        file_identity: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(selection_error)
    }

    fn save(path: &Path, root: &str, storage: &Storage) -> Result<(), StorageError> {
        fs::create_dir_all(
            path.parent()
                .expect("selection file is in an app directory"),
        )
        .map_err(selection_error)?;
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(selection_error)?;
        connection
            .execute_batch("PRAGMA busy_timeout = 5000; PRAGMA synchronous = FULL;")
            .map_err(selection_error)?;
        let tx = connection.transaction().map_err(selection_error)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS last_workspace (
                 singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                 root TEXT NOT NULL, capture_dir TEXT NOT NULL,
                 workspace_id TEXT NOT NULL, file_identity TEXT NOT NULL
             );",
        )
        .map_err(selection_error)?;
        tx.execute(
            "INSERT INTO last_workspace (singleton, root, capture_dir, workspace_id, file_identity)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(singleton) DO UPDATE SET root = excluded.root,
                 capture_dir = excluded.capture_dir, workspace_id = excluded.workspace_id,
                 file_identity = excluded.file_identity",
            params![
                root,
                storage.capture_dir(),
                storage.workspace_id(),
                storage.file_identity()
            ],
        )
        .map_err(selection_error)?;
        tx.commit().map_err(selection_error)
    }
}

fn selection_error(detail: impl std::fmt::Display) -> StorageError {
    error(
        "WRITE_FAILED",
        "workspace_selection",
        None,
        true,
        "无法读取或保存上次工作区设置",
        detail,
    )
}

#[derive(Default)]
pub(crate) struct WorkspaceState {
    generation: u64,
    active: Option<Storage>,
    root: String,
}

impl WorkspaceState {
    fn activate(
        &mut self,
        selection_file: &Path,
        root: &str,
        capture_dir: &str,
        expected: Option<&WorkspaceSelection>,
    ) -> Result<OpenedWorkspaceDto, StorageError> {
        let root_path = Path::new(root);
        let capture_path = Path::new(if capture_dir.is_empty() {
            "."
        } else {
            capture_dir
        });
        let mut opened = None;
        let storage = match self.active.as_mut() {
            Some(storage) if storage.matches_root(root_path)? => {
                if let Some(expected) = expected {
                    storage.verify_capture_dir(capture_path)?;
                    storage.verify_identity(&expected.workspace_id, &expected.file_identity)?;
                } else {
                    storage.update_capture_dir(capture_path)?;
                }
                storage
            }
            _ => opened.insert(match expected {
                Some(expected) => Storage::reopen(
                    root_path,
                    capture_path,
                    STATE_DIR,
                    &expected.workspace_id,
                    &expected.file_identity,
                )?,
                None => Storage::open(root_path, capture_path, STATE_DIR)?,
            }),
        };

        let generation = self.generation + 1;
        let mut view = workspace_view(storage, generation.to_string(), root)?;
        for (id, state, message) in storage.recover_wiki_changes()? {
            if state != "applied" {
                view.recovery_errors.push(RecoveryErrorDto {
                    request_id: id,
                    message,
                });
            }
        }
        if expected.is_none() {
            WorkspaceSelection::save(selection_file, root, storage)?;
        }
        if let Some(storage) = opened {
            self.active = Some(storage);
        }
        self.generation = generation;
        self.root = root.to_owned();
        Ok(view)
    }

    fn resume(
        &mut self,
        selection_file: &Path,
    ) -> Result<Option<OpenedWorkspaceDto>, StorageError> {
        WorkspaceSelection::load(selection_file)?
            .map(|selection| {
                self.activate(
                    selection_file,
                    &selection.root,
                    &selection.capture_dir,
                    Some(&selection),
                )
            })
            .transpose()
    }

    fn workspace(&mut self, session: &str) -> Result<&mut Storage, AppError> {
        self.active
            .as_mut()
            .filter(|_| session == self.generation.to_string())
            .ok_or_else(|| {
                AppError::from_message(
                    "SESSION_EXPIRED",
                    "workspace",
                    false,
                    "工作区会话已失效，请重新打开工作区",
                )
            })
    }

    fn load_capture(&mut self) -> Result<OpenedWorkspaceDto, AppError> {
        let storage = self
            .active
            .as_mut()
            .ok_or_else(|| AppError::invalid("workspace", "请先在主窗口打开工作区"))?;
        workspace_view(storage, self.generation.to_string(), &self.root).map_err(AppError::from)
    }

    fn read_active(&self) -> Result<OpenedWorkspaceDto, AppError> {
        let storage = self
            .active
            .as_ref()
            .ok_or_else(|| AppError::invalid("workspace", "请先在主窗口打开工作区"))?;
        let pending = storage.current_draft_intent(CAPTURE_DRAFT)?;
        let (revision, body) = storage.draft_state(CAPTURE_DRAFT)?;
        Ok(OpenedWorkspaceDto {
            session: self.generation.to_string(),
            root: self.root.clone(),
            capture_dir: storage.capture_dir().to_owned(),
            draft: DraftDto { revision, body },
            recovered: Vec::new(),
            recovery_errors: Vec::new(),
            pending_request_id: pending.map(|(request, ..)| request),
        })
    }
}

fn workspace_view(
    storage: &mut Storage,
    session: String,
    root: &str,
) -> Result<OpenedWorkspaceDto, StorageError> {
    let mut recovered = Vec::new();
    let mut failures = Vec::new();
    for RecoveryOutcome { request_id, result } in storage.recover_pending()? {
        match result {
            Ok(record) => recovered.push(record.relative_path),
            Err(error) => failures.push(RecoveryErrorDto {
                request_id,
                message: error.safe_message.to_owned(),
            }),
        }
    }
    let mut pending = None;
    if let Some((request_id, path, committed)) = storage.current_draft_intent(CAPTURE_DRAFT)? {
        if committed {
            if !recovered.contains(&path) {
                recovered.push(path);
            }
            if let Err(error) = storage.clear_committed_draft(&request_id) {
                failures.push(RecoveryErrorDto {
                    request_id: request_id.clone(),
                    message: error.safe_message.to_owned(),
                });
                pending = Some(request_id);
            }
        } else {
            pending = Some(request_id);
        }
    }
    let (revision, body) = storage.draft_state(CAPTURE_DRAFT)?;
    Ok(OpenedWorkspaceDto {
        session,
        root: root.to_owned(),
        capture_dir: storage.capture_dir().to_owned(),
        draft: DraftDto { revision, body },
        recovered,
        recovery_errors: failures,
        pending_request_id: pending,
    })
}

fn selection_file(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_local_data_dir()
        .map(|path| path.join(SELECTION_FILE))
        .map_err(|error| AppError::from(selection_error(error)))
}

async fn in_main_window<T: Send + 'static>(
    window: WebviewWindow,
    action: impl FnOnce(&mut WorkspaceState, &AppHandle) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    if window.label() != MAIN_WINDOW {
        return Err(AppError::invalid("workspace", "请在主窗口操作工作区"));
    }
    in_workspace_state(window.app_handle(), move |state, app| {
        app.state::<DesktopWindows>()
            .allow_main_action()
            .map_err(|msg| AppError::from_message("WINDOW_BUSY", "window", true, msg))?;
        action(state, app)
    })
    .await
}

async fn in_workspace_state<T: Send + 'static>(
    app: &AppHandle,
    action: impl FnOnce(&mut WorkspaceState, &AppHandle) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let app = app.clone();
    async_runtime::spawn_blocking(move || {
        let state = app.state::<Mutex<WorkspaceState>>();
        let mut state = state.lock().map_err(|_| {
            AppError::from_message(
                "INTERNAL_ERROR",
                "workspace",
                false,
                "工作区状态不可用，请重启应用",
            )
        })?;
        app.state::<DesktopWindows>()
            .allow_window_action()
            .map_err(|msg| AppError::from_message("WINDOW_BUSY", "window", true, msg))?;
        action(&mut state, &app)
    })
    .await
    .map_err(|_| {
        AppError::from_message(
            "CANCELLED",
            "workspace",
            true,
            "工作区操作中断，请重启应用检查保存结果",
        )
    })?
}

pub(crate) async fn with_workspace<T: Send + 'static>(
    window: WebviewWindow,
    session: String,
    action: impl FnOnce(&mut Storage) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    in_main_window(window, move |state, _| action(state.workspace(&session)?)).await
}

async fn with_draft_workspace<T: Send + 'static>(
    window: WebviewWindow,
    session: String,
    action: impl FnOnce(&mut Storage) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let capture = match window.label() {
        MAIN_WINDOW => false,
        CAPTURE_WINDOW => true,
        _ => return Err(AppError::invalid("draft", "请在记录窗口操作草稿")),
    };
    in_workspace_state(window.app_handle(), move |state, app| {
        if capture != app.state::<DesktopWindows>().capture_active() {
            return Err(AppError::invalid("draft", "请在当前记录窗口操作草稿"));
        }
        action(state.workspace(&session)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn open_workspace(
    window: WebviewWindow,
    root: String,
    capture_dir: String,
) -> Result<OpenedWorkspaceDto, AppError> {
    in_main_window(window, move |state, app| {
        app.state::<DesktopWindows>()
            .allow_workspace_switch()
            .map_err(|msg| AppError::from_message("WINDOW_BUSY", "workspace", true, msg))?;
        state
            .activate(&selection_file(app)?, &root, &capture_dir, None)
            .map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn resume_workspace(
    window: WebviewWindow,
) -> Result<Option<OpenedWorkspaceDto>, AppError> {
    in_main_window(window, move |state, app| {
        if app.state::<DesktopWindows>().capture_active() {
            return state.read_active().map(Some);
        }
        state.resume(&selection_file(app)?).map_err(AppError::from)
    })
    .await
}

fn window_error(detail: tauri::Error) -> AppError {
    eprintln!("窗口操作失败（{detail}）");
    AppError::new(
        "WINDOW_FAILED",
        "window",
        true,
        "窗口操作失败，请重试",
        Some(detail.to_string()),
    )
}

pub(crate) fn init_tray(app: &AppHandle) -> tauri::Result<()> {
    let capture = MenuItem::with_id(app, TRAY_CAPTURE, "新建笔记", true, None::<&str>)?;
    let workbench = MenuItem::with_id(app, TRAY_WORKBENCH, "打开工作台", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, TRAY_EXIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&capture, &workbench, &exit])?;
    let icon = app
        .default_window_icon()
        .ok_or_else(|| std::io::Error::other("缺少托盘图标"))?;
    TrayIconBuilder::new()
        .menu(&menu)
        .icon(icon.clone())
        .tooltip(&app.package_info().name)
        .on_menu_event(on_tray_menu)
        .build(app)?;
    Ok(())
}

fn on_tray_menu(app: &AppHandle, event: MenuEvent) {
    if event.id().as_ref() == TRAY_CAPTURE {
        show_from_resident(app);
        return;
    }
    let app = app.clone();
    async_runtime::spawn(async move {
        let result = match event.id().as_ref() {
            TRAY_WORKBENCH => {
                let windows = app.state::<DesktopWindows>();
                let _transition = windows.transition.lock().await;
                show_workbench(&app)
            }
            TRAY_EXIT => request_exit(&app).await,
            _ => return,
        };
        if let Err(error) = result {
            eprintln!("托盘操作失败（{}）", error.safe_message);
        }
    });
}

fn show_workbench(app: &AppHandle) -> Result<(), AppError> {
    app.state::<DesktopWindows>()
        .allow_window_action()
        .map_err(|msg| AppError::from_message("WINDOW_BUSY", "window", true, msg))?;
    let window = match app.get_webview_window(MAIN_WINDOW) {
        Some(window) => window,
        None => build_window(app, MAIN_WINDOW)?,
    };
    focus_window(&window)
}

async fn request_exit(app: &AppHandle) -> Result<(), AppError> {
    let windows = app.state::<DesktopWindows>();
    let _transition = windows.transition.lock().await;
    if windows.capture_active() {
        let capture = app.get_webview_window(CAPTURE_WINDOW).ok_or_else(|| {
            AppError::from_message("WINDOW_BUSY", "window", true, "快捷窗正在打开，请稍后退出")
        })?;
        let focus = focus_window(&capture);
        app.emit_to(CAPTURE_WINDOW, "exit-requested", ())
            .map_err(window_error)?;
        focus
    } else {
        begin_exit(app).await?;
        app.exit(0);
        Ok(())
    }
}

async fn begin_exit(app: &AppHandle) -> Result<(), AppError> {
    in_workspace_state(app, |_, app| {
        app.state::<DesktopWindows>().begin_exit();
        Ok(())
    })
    .await
}

fn focus_window(window: &WebviewWindow) -> Result<(), AppError> {
    window.unminimize().map_err(window_error)?;
    window.set_focus().map_err(window_error)
}

fn build_window(app: &AppHandle, label: &str) -> Result<WebviewWindow, AppError> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == label)
        .ok_or_else(|| AppError::invalid("window", format!("缺少 {label} 窗口配置")))?;
    // A Windows WebView may wait for the event loop. Nothing here holds the storage lock.
    WebviewWindowBuilder::from_config(app, config)
        .and_then(WebviewWindowBuilder::build)
        .map_err(window_error)
}

pub(crate) fn show_from_resident(app: &AppHandle) {
    let previous_focus = ForegroundWindow::current();
    let app = app.clone();
    async_runtime::spawn(async move {
        if let Err(error) = open_capture(&app, None, previous_focus).await {
            eprintln!("无法唤出快捷窗（{}）", error.safe_message);
            let windows = app.state::<DesktopWindows>();
            let _transition = windows.transition.lock().await;
            if !windows.capture_active() {
                if let Err(error) = show_workbench(&app) {
                    eprintln!("无法打开工作台（{}）", error.safe_message);
                }
            }
        }
    });
}

#[tauri::command]
pub(crate) async fn show_capture_window(
    window: WebviewWindow,
    session: String,
) -> Result<(), AppError> {
    if window.label() != MAIN_WINDOW {
        return Err(AppError::invalid("window", "请在主窗口打开快捷窗"));
    }
    let focus = ForegroundWindow::remember(window.hwnd().map_err(window_error)?.0);
    open_capture(window.app_handle(), Some(session), focus).await
}

async fn open_capture(
    app: &AppHandle,
    session: Option<String>,
    previous_focus: Option<ForegroundWindow>,
) -> Result<(), AppError> {
    let windows = app.state::<DesktopWindows>();
    let _transition = windows.transition.lock().await;
    let create = in_workspace_state(app, move |state, app| {
        if let Some(session) = session {
            state.workspace(&session)?;
        } else if state.active.is_none()
            && state
                .resume(&selection_file(app)?)
                .map_err(AppError::from)?
                .is_none()
        {
            return Ok(None);
        }
        app.state::<DesktopWindows>()
            .begin_capture()
            .map(Some)
            .map_err(|msg| AppError::from_message("WINDOW_BUSY", "window", true, msg))
    })
    .await?;
    let Some(create) = create else {
        return show_workbench(app);
    };
    if !create {
        let capture = app.get_webview_window(CAPTURE_WINDOW).ok_or_else(|| {
            AppError::from_message(
                "WINDOW_BUSY",
                "window",
                true,
                "快捷窗正在打开或关闭，请重试",
            )
        })?;
        focus_window(&capture)?;
        return app
            .emit_to(CAPTURE_WINDOW, "capture-focus-editor", ())
            .map_err(window_error);
    }

    *windows.previous_focus.lock().unwrap() = previous_focus;
    if let Some(capture) = app.get_webview_window(CAPTURE_WINDOW) {
        let result = in_workspace_state(app, |state, _| state.load_capture())
            .await
            .and_then(|view| {
                app.emit_to(CAPTURE_WINDOW, "capture-opened", view)
                    .map_err(window_error)
            })
            .and_then(|()| capture.show().map_err(window_error))
            .and_then(|()| focus_window(&capture))
            .and_then(|()| {
                app.emit_to(CAPTURE_WINDOW, "capture-focus-editor", ())
                    .map_err(window_error)
            });
        if result.is_err() {
            if let Err(error) = capture.hide() {
                eprintln!("无法隐藏打开失败的快捷窗（{error}）");
            }
            windows.capture_closed();
        }
        return result;
    }
    match build_window(app, CAPTURE_WINDOW) {
        Ok(capture) => focus_window(&capture),
        Err(error) => {
            windows.capture_closed();
            Err(error)
        }
    }
}

fn require_capture_window(window: &WebviewWindow) -> Result<(), AppError> {
    if window.label() != CAPTURE_WINDOW {
        return Err(AppError::invalid("draft", "请在快捷窗操作草稿"));
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn load_capture_workspace(
    window: WebviewWindow,
) -> Result<OpenedWorkspaceDto, AppError> {
    require_capture_window(&window)?;
    in_workspace_state(window.app_handle(), |state, app| {
        if !app.state::<DesktopWindows>().capture_active() {
            return Err(AppError::from_message(
                "WINDOW_CLOSED",
                "window",
                false,
                "快捷窗已关闭",
            ));
        }
        state.load_capture()
    })
    .await
}

#[tauri::command]
pub(crate) async fn close_capture_window(
    window: WebviewWindow,
    exit: Option<bool>,
) -> Result<(), AppError> {
    require_capture_window(&window)?;
    let app = window.app_handle();
    let windows = app.state::<DesktopWindows>();
    let _transition = windows.transition.lock().await;
    if !windows.capture_active() {
        return Err(AppError::from_message(
            "WINDOW_CLOSED",
            "window",
            false,
            "快捷窗已关闭",
        ));
    }
    if exit == Some(true) {
        begin_exit(app).await?;
    }
    let previous_focus = *windows.previous_focus.lock().unwrap();
    if let Some(previous) = previous_focus {
        let result = window.hwnd().and_then(|capture| {
            let handle = capture.0 as isize;
            window.run_on_main_thread(move || previous.restore(handle))
        });
        if let Err(error) = result {
            eprintln!("无法交还窗口焦点（{error}）");
        }
    }
    if exit == Some(true) {
        app.exit(0);
        Ok(())
    } else {
        window.hide().map_err(window_error)?;
        capture_closed(app);
        Ok(())
    }
}

#[tauri::command]
pub(crate) async fn list_recent_captures(
    window: WebviewWindow,
    session: String,
) -> Result<Vec<RecentCaptureDto>, AppError> {
    with_workspace(window, session, |storage| {
        let recent = storage.list_recent_captures()?;
        Ok(recent
            .into_iter()
            .map(|(path, captured_at, body, content_hash)| RecentCaptureDto {
                path,
                captured_at,
                body,
                content_hash,
                is_capture: true,
            })
            .collect())
    })
    .await
}

pub(crate) async fn with_search_reader<T: Send + 'static>(
    window: WebviewWindow,
    session: String,
    action: impl FnOnce(SearchReader) -> Result<T, StorageError> + Send + 'static,
) -> Result<T, AppError> {
    let reader = with_workspace(window.clone(), session.clone(), |storage| {
        storage.search_reader().map_err(AppError::from)
    })
    .await?;
    // File enumeration must not hold the mutex used by quick capture.
    let result = async_runtime::spawn_blocking(move || action(reader))
        .await
        .map_err(|_| {
            AppError::from_message("CANCELLED", "search", true, "索引或原文读取中断，请重试")
        })?;
    with_workspace(window, session, |_| result.map_err(AppError::from)).await
}

#[tauri::command]
pub(crate) async fn search_records(
    window: WebviewWindow,
    session: String,
    query: String,
    from_utc: Option<String>,
    until_utc: Option<String>,
    offset: u32,
) -> Result<SearchPageDto, AppError> {
    with_search_reader(window, session, move |mut reader| {
        let (hits, has_more, skipped) =
            reader.search_records(&query, from_utc.as_deref(), until_utc.as_deref(), offset)?;
        let hits = hits
            .into_iter()
            .map(|(path, captured_at, content_hash, preview, title_text)| {
                let is_capture = reader.is_capture_path(&path);
                SearchHitDto {
                    path,
                    captured_at,
                    content_hash,
                    preview,
                    title_text,
                    is_capture,
                }
            })
            .collect();
        Ok(SearchPageDto {
            hits,
            has_more,
            skipped,
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn rebuild_search_index(
    window: WebviewWindow,
    session: String,
) -> Result<IndexRebuildDto, AppError> {
    with_search_reader(window, session, |mut reader| {
        let (indexed, skipped) = reader.rebuild_search_index()?;
        Ok(IndexRebuildDto { indexed, skipped })
    })
    .await
}

#[tauri::command]
pub(crate) async fn open_record_source(
    window: WebviewWindow,
    session: String,
    relative_path: String,
    content_hash: String,
) -> Result<RecordSourceDto, AppError> {
    with_search_reader(window, session, move |reader| {
        let (state, content_hash, body) =
            reader.read_record_source(&relative_path, &content_hash)?;
        let is_capture = reader.is_capture_path(&relative_path);
        Ok(RecordSourceDto {
            state,
            content_hash,
            body,
            is_capture,
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn persist_draft(
    window: WebviewWindow,
    session: String,
    revision: i64,
    body: String,
) -> Result<i64, AppError> {
    with_draft_workspace(window, session, move |storage| {
        storage
            .save_draft(CAPTURE_DRAFT, revision, &body)
            .map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn commit_capture(
    window: WebviewWindow,
    session: String,
    request_id: String,
    revision: i64,
    body: String,
) -> Result<String, AppError> {
    with_draft_workspace(window, session, move |storage| {
        storage
            .save_record(&request_id, CAPTURE_DRAFT, revision, &body)
            .map(|record| record.relative_path)
            .map_err(AppError::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn confirm_capture(
    window: WebviewWindow,
    session: String,
    request_id: String,
) -> Result<bool, AppError> {
    with_draft_workspace(window, session, move |storage| {
        storage
            .clear_committed_draft(&request_id)
            .map_err(AppError::from)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::TestDir;

    #[test]
    fn active_capture_allows_main_close_and_prevents_workspace_switch() {
        let desktop = DesktopWindows::default();
        assert_eq!(desktop.begin_capture(), Ok(true));
        assert_eq!(desktop.begin_capture(), Ok(false));
        assert!(desktop.allow_workspace_switch().is_err());
        assert!(desktop.begin_main_close());
        assert!(desktop.allow_main_action().is_err());
        assert!(desktop.allow_window_action().is_ok());
        desktop.main_closed();
        assert!(desktop.allow_main_action().is_ok());
        desktop.capture_closed();
        assert!(desktop.allow_workspace_switch().is_ok());
        assert_eq!(desktop.begin_capture(), Ok(true));
        desktop.begin_exit();
        desktop.capture_closed();
        assert!(desktop.begin_capture().is_err());
    }

    #[test]
    fn capture_load_reuses_the_session_and_recovers_an_unconfirmed_commit() {
        let workspace = TestDir::new();
        let data = TestDir::new();
        let file = data.0.join(SELECTION_FILE);
        let root = workspace.0.to_str().unwrap();
        let mut state = WorkspaceState::default();
        let opened = state.activate(&file, root, "", None).unwrap();
        let session = opened.session;
        state
            .workspace(&session)
            .unwrap()
            .save_draft(CAPTURE_DRAFT, 1, "快捷窗草稿")
            .unwrap();
        let capture_view = state.load_capture().unwrap();
        assert_eq!(capture_view.session, session);
        assert_eq!(capture_view.root, root);
        assert_eq!(capture_view.draft.revision, 1);
        assert_eq!(capture_view.draft.body.as_deref(), Some("快捷窗草稿"));

        let record = state
            .workspace(&capture_view.session)
            .unwrap()
            .save_record("quick-capture-request", CAPTURE_DRAFT, 1, "快捷窗草稿")
            .unwrap();
        let active_view = state.read_active().unwrap();
        assert_eq!(active_view.session, capture_view.session);
        assert_eq!(active_view.draft.revision, 1);
        assert_eq!(active_view.draft.body.as_deref(), Some("快捷窗草稿"));
        assert!(active_view.recovered.is_empty());
        assert!(active_view.recovery_errors.is_empty());
        assert_eq!(
            active_view.pending_request_id.as_deref(),
            Some("quick-capture-request")
        );
        let reloaded = state.load_capture().unwrap();
        assert_eq!(reloaded.session, session);
        assert!(state.workspace(&session).is_ok());
        assert_eq!(reloaded.draft.revision, 1);
        assert_eq!(reloaded.draft.body, None);
        assert_eq!(reloaded.recovered, [record.relative_path]);
        assert!(reloaded.recovery_errors.is_empty());
        assert!(reloaded.pending_request_id.is_none());
    }

    #[test]
    fn resumes_unacknowledged_commit_and_keeps_newer_draft() {
        let workspace = TestDir::new();
        let data = TestDir::new();
        let file = data.0.join(SELECTION_FILE);
        let root = workspace.0.to_str().unwrap();

        for newer in [false, true] {
            let mut state = WorkspaceState::default();
            let opened = state.activate(&file, root, "", None).unwrap();
            let session = opened.session;
            let base_revision = opened.draft.revision;
            let storage = state.workspace(&session).unwrap();
            let revision = base_revision + 1;
            storage
                .save_draft(CAPTURE_DRAFT, revision, "合成草稿")
                .unwrap();
            let request_id = format!("request-{revision}");
            let record = storage
                .save_record(&request_id, CAPTURE_DRAFT, revision, "合成草稿")
                .unwrap();
            if newer {
                storage
                    .save_draft(CAPTURE_DRAFT, revision + 1, "后来编辑")
                    .unwrap();
            }
            drop(state);

            let mut state = WorkspaceState::default();
            let resumed = state.resume(&file).unwrap().unwrap();
            if newer {
                assert_eq!(resumed.draft.revision, revision + 1);
                assert_eq!(resumed.draft.body.as_deref(), Some("后来编辑"));
                assert!(resumed.recovered.is_empty());
            } else {
                assert_eq!(resumed.draft.revision, revision);
                assert_eq!(resumed.draft.body, None);
                assert_eq!(resumed.recovered, [record.relative_path.clone()]);
            }
            assert!(resumed.recovery_errors.is_empty());
            assert!(resumed.pending_request_id.is_none());
            assert!(workspace.0.join(&record.relative_path).is_file());
            assert_eq!(
                state
                    .workspace(&resumed.session)
                    .unwrap()
                    .save_record(&request_id, CAPTURE_DRAFT, revision, "合成草稿")
                    .unwrap(),
                record
            );
        }
    }

    #[test]
    fn failed_recovery_reuses_the_request_after_reopen() {
        let workspace = TestDir::new();
        let data = TestDir::new();
        let file = data.0.join(SELECTION_FILE);
        let root = workspace.0.to_str().unwrap();
        let mut state = WorkspaceState::default();
        let session = state.activate(&file, root, "", None).unwrap().session;
        let connection =
            Connection::open(workspace.0.join(STATE_DIR).join("state.sqlite")).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_commit BEFORE INSERT ON records
                 BEGIN SELECT RAISE(FAIL, 'synthetic commit failure'); END;",
            )
            .unwrap();
        let storage = state.workspace(&session).unwrap();
        storage.save_draft(CAPTURE_DRAFT, 1, "合成草稿").unwrap();
        assert_eq!(
            storage
                .save_record("request", CAPTURE_DRAFT, 1, "合成草稿")
                .unwrap_err()
                .code,
            "WRITE_FAILED"
        );
        drop(state);

        let mut state = WorkspaceState::default();
        let resumed = state.resume(&file).unwrap().unwrap();
        assert_eq!(resumed.draft.revision, 1);
        assert_eq!(resumed.draft.body.as_deref(), Some("合成草稿"));
        assert!(resumed.recovered.is_empty());
        assert_eq!(
            resumed.recovery_errors,
            [RecoveryErrorDto {
                request_id: "request".to_owned(),
                message: "本地状态库操作失败".to_owned(),
            }]
        );
        assert_eq!(resumed.pending_request_id.as_deref(), Some("request"));

        connection
            .execute_batch("DROP TRIGGER fail_commit")
            .unwrap();
        let storage = state.workspace(&resumed.session).unwrap();
        let record = storage
            .save_record(
                resumed.pending_request_id.as_deref().unwrap(),
                CAPTURE_DRAFT,
                1,
                "合成草稿",
            )
            .unwrap();
        assert!(workspace.0.join(&record.relative_path).is_file());
        assert!(storage.clear_committed_draft("request").unwrap());
        assert_eq!(storage.draft_state(CAPTURE_DRAFT).unwrap(), (1, None));
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM records", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn switching_revokes_the_old_session_and_resume_requires_the_saved_workspace() {
        let first = TestDir::new();
        let second = TestDir::new();
        let data = TestDir::new();
        let file = data.0.join(SELECTION_FILE);
        let first_workspace = first.0.join("workspace");
        fs::create_dir(&first_workspace).unwrap();
        let first_root = first_workspace.to_str().unwrap();
        let second_root = second.0.to_str().unwrap();
        let mut state = WorkspaceState::default();
        assert!(state.resume(&file).unwrap().is_none());
        let first_session = state.activate(&file, first_root, "", None).unwrap().session;
        let refreshed_session = state.activate(&file, first_root, "", None).unwrap().session;
        assert!(state.workspace(&first_session).is_err());
        assert!(state.activate(&file, second_root, "missing", None).is_err());
        assert!(state.workspace(&refreshed_session).is_ok());
        let second_session = state
            .activate(&file, second_root, "", None)
            .unwrap()
            .session;
        let latest_session = state.activate(&file, first_root, "", None).unwrap().session;
        assert!(state.workspace(&refreshed_session).is_err());
        assert!(state.workspace(&second_session).is_err());
        assert!(state.workspace(&latest_session).is_ok());
        drop(state);

        let state_dir = first_workspace.join(STATE_DIR);
        let moved = first_workspace.join("old-state");
        fs::rename(&state_dir, &moved).unwrap();
        let mut state = WorkspaceState::default();
        assert!(state.resume(&file).is_err());
        assert!(!state_dir.exists());
        let replacement = Storage::open(&first_workspace, Path::new("."), STATE_DIR).unwrap();
        drop(replacement);
        assert!(state.resume(&file).is_err());
        assert!(state.active.is_none());

        let moved_root = first.0.join("old-root");
        fs::rename(&first_workspace, &moved_root).unwrap();
        fs::create_dir(&first_workspace).unwrap();
        fs::rename(moved_root.join("old-state"), &state_dir).unwrap();
        assert!(state.resume(&file).is_err());
    }
}
