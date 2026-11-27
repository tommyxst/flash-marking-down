use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::async_runtime::{self, Mutex as AsyncMutex};
use tauri::menu::{Menu, MenuEvent, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{
    AppHandle, Emitter, Listener, Manager, WebviewWindow, WebviewWindowBuilder, Window, WindowEvent,
};

use crate::dto::wikra::CaptureCommitResultDto;
use crate::dto::{
    AppError, DraftDto, IndexRebuildDto, OpenedWorkspaceDto, RecentCaptureDto, RecordSourceDto,
    RecoveryErrorDto, SearchHitDto, SearchPageDto,
};
use crate::platform::ForegroundWindow;
use crate::settings;
use crate::storage::wikra::organize::CaptureMode;
use crate::storage::{SearchReader, Storage, StorageError, WorkspaceStorage, error};
use crate::workspace::{DEFAULT_CAPTURE_DIR, STANDARD_CAPTURE_DIR, STATE_DIR};

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

#[derive(Default)]
pub(crate) struct WorkspaceState {
    generation: u64,
    active: Option<WorkspaceStorage>,
    root: String,
}

impl WorkspaceState {
    fn activate(
        &mut self,
        settings_file: &Path,
        root: &str,
        capture_dir: &str,
        remember: bool,
    ) -> Result<OpenedWorkspaceDto, StorageError> {
        let root_path = Path::new(root);
        let capture_path = Path::new(if capture_dir.trim().is_empty() {
            DEFAULT_CAPTURE_DIR
        } else {
            capture_dir
        });
        let storage = Storage::open(root_path, capture_path, STATE_DIR)?;
        self.activate_storage(settings_file, root, capture_dir, remember, storage)
    }

    fn activate_storage(
        &mut self,
        settings_file: &Path,
        root: &str,
        capture_dir: &str,
        remember: bool,
        mut storage: Storage,
    ) -> Result<OpenedWorkspaceDto, StorageError> {
        let generation = self.generation + 1;
        let view = workspace_view(&mut storage, generation.to_string(), root)?;
        if remember || capture_dir != storage.capture_dir() {
            settings::update(settings_file, |settings| {
                settings.last_workspace_root = Some(root.to_owned());
                settings.last_capture_dir = storage.capture_dir().to_owned();
            })?;
        }
        self.active = Some(storage.into_workspace());
        self.generation = generation;
        self.root = root.to_owned();
        Ok(view)
    }

    fn resume(&mut self, settings_file: &Path) -> Result<Option<OpenedWorkspaceDto>, StorageError> {
        let settings = settings::read(settings_file)?;
        settings
            .last_workspace_root
            .as_deref()
            .map(|root| self.activate(settings_file, root, &settings.last_capture_dir, false))
            .transpose()
    }

    fn workspace(&mut self, session: &str) -> Result<&mut WorkspaceStorage, AppError> {
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

    fn update_dirs(
        &mut self,
        settings_file: &Path,
        session: &str,
        raw_dir: &str,
        capture_dir: Option<&str>,
    ) -> Result<OpenedWorkspaceDto, AppError> {
        if raw_dir.trim().is_empty() {
            return Err(AppError::invalid("workspace", "请填写原始文件目录"));
        }
        let capture_dir = capture_dir
            .filter(|dir| !dir.trim().is_empty())
            .map(Path::new);
        let active = self.workspace(session)?;
        let (raw_dir, capture_dir) = active
            .with_storage(|storage| storage.set_workspace_dirs(Path::new(raw_dir), capture_dir))?;
        active.update_dirs(raw_dir, capture_dir.clone());
        settings::update(settings_file, |settings| {
            settings.last_capture_dir = capture_dir;
        })?;
        self.read_active()
    }

    fn load_capture(&mut self) -> Result<OpenedWorkspaceDto, AppError> {
        let storage = self
            .active
            .as_mut()
            .ok_or_else(|| AppError::invalid("workspace", "请先在主窗口打开工作区"))?;
        storage.with_storage(|storage| {
            workspace_view(storage, self.generation.to_string(), &self.root).map_err(AppError::from)
        })
    }

    fn read_active(&mut self) -> Result<OpenedWorkspaceDto, AppError> {
        let storage = self
            .active
            .as_mut()
            .ok_or_else(|| AppError::invalid("workspace", "请先在主窗口打开工作区"))?;
        storage.with_storage(|storage| {
            let pending = storage.committed_draft(CAPTURE_DRAFT)?;
            let (revision, body) = storage.draft_state(CAPTURE_DRAFT)?;
            Ok(OpenedWorkspaceDto {
                session: self.generation.to_string(),
                root: self.root.clone(),
                raw_dir: storage.raw_dir().to_owned(),
                capture_dir: storage.capture_dir().to_owned(),
                draft: DraftDto { revision, body },
                recovered: Vec::new(),
                recovery_errors: Vec::new(),
                pending_request_id: pending.map(|(request, ..)| request),
            })
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
    let mut pending = None;
    if let Some((request_id, path)) = storage.committed_draft(CAPTURE_DRAFT)? {
        match storage.clear_committed_draft(&request_id) {
            Ok(_) => recovered.push(path),
            Err(error) => {
                failures.push(RecoveryErrorDto {
                    request_id: request_id.clone(),
                    message: error.safe_message.to_owned(),
                });
                pending = Some(request_id);
            }
        }
    }
    let (revision, body) = storage.draft_state(CAPTURE_DRAFT)?;
    Ok(OpenedWorkspaceDto {
        session,
        root: root.to_owned(),
        raw_dir: storage.raw_dir().to_owned(),
        capture_dir: storage.capture_dir().to_owned(),
        draft: DraftDto { revision, body },
        recovered,
        recovery_errors: failures,
        pending_request_id: pending,
    })
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

pub(crate) fn active_workspace(app: &AppHandle) -> Result<Option<WorkspaceStorage>, AppError> {
    let state = app.state::<Mutex<WorkspaceState>>();
    let state = state.lock().map_err(|_| {
        AppError::from_message(
            "INTERNAL_ERROR",
            "workspace",
            false,
            "工作区状态不可用，请重启应用",
        )
    })?;
    Ok(state.active.clone())
}

pub(crate) async fn with_workspace<T: Send + 'static>(
    window: WebviewWindow,
    session: String,
    action: impl FnOnce(&mut Storage) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    in_main_window(window, move |state, _| {
        state.workspace(&session)?.with_storage(action)
    })
    .await
}

pub(crate) async fn workspace_for_session(
    window: WebviewWindow,
    session: String,
) -> Result<WorkspaceStorage, AppError> {
    in_main_window(window, move |state, _| {
        Ok(state.workspace(&session)?.clone())
    })
    .await
}

fn wikilink_window_allowed(label: &str, capture_active: bool) -> bool {
    label == MAIN_WINDOW || label == CAPTURE_WINDOW && capture_active
}

#[tauri::command]
pub(crate) async fn list_wikilink_candidates(
    window: WebviewWindow,
    session: String,
) -> Result<Vec<crate::dto::wikra::WikilinkCandidateDto>, AppError> {
    let label = window.label().to_owned();
    in_workspace_state(window.app_handle(), move |state, app| {
        if !wikilink_window_allowed(&label, app.state::<DesktopWindows>().capture_active()) {
            return Err(AppError::invalid(
                "wikilink",
                "请在主窗口或活动的快捷窗口补全双链",
            ));
        }
        state
            .workspace(&session)?
            .with_storage(|storage| Ok(storage.list_wikilink_candidates()?))
    })
    .await
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
        state.workspace(&session)?.with_storage(action)
    })
    .await
}

#[tauri::command]
pub(crate) async fn open_workspace(
    window: WebviewWindow,
    root: String,
    capture_dir: String,
) -> Result<OpenedWorkspaceDto, AppError> {
    open_selected_workspace(window, root, capture_dir, false).await
}

#[tauri::command]
pub(crate) async fn init_standard_vault(
    window: WebviewWindow,
    root: String,
) -> Result<OpenedWorkspaceDto, AppError> {
    open_selected_workspace(window, root, STANDARD_CAPTURE_DIR.to_owned(), true).await
}

async fn open_selected_workspace(
    window: WebviewWindow,
    root: String,
    capture_dir: String,
    standard_vault: bool,
) -> Result<OpenedWorkspaceDto, AppError> {
    if window.label() == MAIN_WINDOW
        && window.state::<DesktopWindows>().capture_active()
        && let Some(capture) = window.get_webview_window(CAPTURE_WINDOW)
    {
        let desktop = window.state::<DesktopWindows>();
        *desktop.previous_focus.lock().unwrap() =
            ForegroundWindow::remember(window.hwnd().map_err(window_error)?.0);
        focus_window(&capture)?;
        let (closed, wait) = std::sync::mpsc::channel();
        let listener = window.once("capture-closed", move |_| {
            let _ = closed.send(());
        });
        let request = capture.emit_to(CAPTURE_WINDOW, "capture-close-requested", ());
        if request.is_ok() && desktop.capture_active() {
            // Allow the draft's SQLite busy timeout to finish; a failed write keeps its editor open.
            const CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
            let _ = async_runtime::spawn_blocking(move || wait.recv_timeout(CLOSE_TIMEOUT)).await;
        }
        window.unlisten(listener);
        request.map_err(window_error)?;
    }
    let app = window.app_handle().clone();
    let opened = in_main_window(window, move |state, app| {
        app.state::<DesktopWindows>()
            .allow_workspace_switch()
            .map_err(|msg| AppError::from_message("WINDOW_BUSY", "workspace", true, msg))?;
        let settings_file = settings::settings_path(app)?;
        if standard_vault {
            let storage = Storage::open(Path::new(&root), Path::new(&capture_dir), STATE_DIR)?;
            storage
                .workspace_files()
                .initialize_standard_layout()
                .map_err(|err| {
                    error(
                        "WRITE_FAILED",
                        "workspace",
                        None,
                        true,
                        "无法初始化标准知识库",
                        err,
                    )
                })?;
            Ok(state.activate_storage(&settings_file, &root, &capture_dir, true, storage)?)
        } else {
            Ok(state.activate(&settings_file, &root, &capture_dir, true)?)
        }
    })
    .await?;
    crate::m2_commands::wake_memory_backfill(&app);
    crate::wikra_commands::wake_active_buffer(&app);
    Ok(opened)
}

#[tauri::command]
pub(crate) async fn update_workspace_dirs(
    window: WebviewWindow,
    session: String,
    raw_dir: String,
    capture_dir: Option<String>,
) -> Result<OpenedWorkspaceDto, AppError> {
    in_main_window(window, move |state, app| {
        app.state::<DesktopWindows>()
            .allow_workspace_switch()
            .map_err(|_| {
                AppError::from_message(
                    "WINDOW_BUSY",
                    "workspace",
                    true,
                    "请先关闭快捷窗，再修改工作区目录",
                )
            })?;
        state.update_dirs(
            &settings::settings_path(app)?,
            &session,
            &raw_dir,
            capture_dir.as_deref(),
        )
    })
    .await
}

#[tauri::command]
pub(crate) async fn resume_workspace(
    window: WebviewWindow,
) -> Result<Option<OpenedWorkspaceDto>, AppError> {
    let app = window.app_handle().clone();
    let opened = in_main_window(window, move |state, app| {
        if state.active.is_some() {
            return state.read_active().map(Some);
        }
        state
            .resume(&settings::settings_path(app)?)
            .map_err(AppError::from)
    })
    .await?;
    if opened.is_some() {
        crate::m2_commands::wake_memory_backfill(&app);
        crate::wikra_commands::wake_active_buffer(&app);
    }
    Ok(opened)
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
        .tooltip("Wikra")
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
                .resume(&settings::settings_path(app)?)
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
    crate::wikra_commands::wake_active_buffer(app);
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
            .map(
                |(path, captured_at, body, content_hash, block_id)| RecentCaptureDto {
                    path,
                    block_id,
                    captured_at,
                    body,
                    content_hash,
                    is_capture: true,
                },
            )
            .collect())
    })
    .await
}

pub(crate) async fn with_search_reader<T: Send + 'static>(
    window: WebviewWindow,
    session: String,
    action: impl FnOnce(SearchReader) -> Result<T, StorageError> + Send + 'static,
) -> Result<T, AppError> {
    let expected = session.clone();
    let reader = in_main_window(window.clone(), move |state, _| {
        state
            .workspace(&expected)?
            .search_reader()
            .map_err(AppError::from)
    })
    .await?;
    // File enumeration must not hold the mutex used by quick capture.
    let result = async_runtime::spawn_blocking(move || action(reader))
        .await
        .map_err(|_| {
            AppError::from_message("CANCELLED", "search", true, "索引或原文读取中断，请重试")
        })?;
    in_main_window(window, move |state, _| {
        state.workspace(&session)?;
        result.map_err(AppError::from)
    })
    .await
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
    let app = window.app_handle().clone();
    let preferences = settings::settings_path(&app)
        .and_then(|path| settings::read(&path))
        .unwrap_or_default();
    let (relative_path, enqueued) = with_draft_workspace(window, session, move |storage| {
        let record = storage.save_record(&request_id, CAPTURE_DRAFT, revision, &body)?;
        let enqueued =
            crate::m2_commands::queue_saved_note(storage, &record.record_id, &preferences);
        Ok((record.relative_path, enqueued))
    })
    .await?;
    if enqueued {
        crate::m2_commands::wake_organize_worker(&app);
    }
    Ok(relative_path)
}

#[tauri::command]
pub(crate) async fn commit_capture_v2(
    window: WebviewWindow,
    session: String,
    request_id: String,
    revision: i64,
    body: String,
    mode: String,
) -> Result<CaptureCommitResultDto, AppError> {
    let app = window.app_handle().clone();
    let preferences = settings::settings_path(&app)
        .and_then(|path| settings::read(&path))
        .unwrap_or_default();
    let mode = CaptureMode::parse(&mode, preferences.buffer_seconds)?;
    let (receipt, workspace) = with_draft_workspace(window, session, move |storage| {
        let record = storage.save_record_with_buffer(
            &request_id,
            CAPTURE_DRAFT,
            revision,
            &body,
            Some(mode),
        )?;
        Ok((storage.capture_receipt(record)?, storage.workspace_handle()))
    })
    .await?;
    if receipt.queued {
        crate::wikra_commands::wake_buffer(&app, workspace, receipt.buffer_seconds_left);
    }
    Ok(receipt)
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
    use std::fs;

    #[test]
    fn wikilink_candidates_accept_main_and_active_capture_for_the_current_session() {
        let directory = TestDir::new();
        let settings = directory.0.join("settings.json");
        let root = directory.0.to_str().unwrap();
        let mut state = WorkspaceState::default();
        let opened = state.activate(&settings, root, "notes", false).unwrap();
        let desktop = DesktopWindows::default();
        assert!(!wikilink_window_allowed(
            CAPTURE_WINDOW,
            desktop.capture_active()
        ));
        desktop.begin_capture().unwrap();
        assert!(wikilink_window_allowed(
            MAIN_WINDOW,
            desktop.capture_active()
        ));
        assert!(wikilink_window_allowed(
            CAPTURE_WINDOW,
            desktop.capture_active()
        ));
        assert!(!wikilink_window_allowed("other", desktop.capture_active()));
        assert!(state.workspace("expired-session").is_err());
        let workspace = state.workspace(&opened.session).unwrap();
        let candidates = workspace
            .with_storage(|storage| {
                storage.create_markdown_document("wiki/Inbox.md", "# Capture lookup\n")?;
                storage.list_wikilink_candidates()
            })
            .unwrap();
        assert_eq!(candidates[0].link_text, "[[Inbox|Capture lookup]]");
    }

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
        let file = data.0.join(settings::FILE_NAME);
        let root = workspace.0.to_str().unwrap();
        let mut state = WorkspaceState::default();
        let opened = state.activate(&file, root, "", true).unwrap();
        let session = opened.session;
        state
            .workspace(&session)
            .unwrap()
            .with_storage(|storage| storage.save_draft(CAPTURE_DRAFT, 1, "快捷窗草稿"))
            .unwrap();
        let capture_view = state.load_capture().unwrap();
        assert_eq!(capture_view.session, session);
        assert_eq!(capture_view.root, root);
        assert_eq!(capture_view.draft.revision, 1);
        assert_eq!(capture_view.draft.body.as_deref(), Some("快捷窗草稿"));

        let record = state
            .workspace(&capture_view.session)
            .unwrap()
            .with_storage(|storage| {
                storage.save_record("quick-capture-request", CAPTURE_DRAFT, 1, "快捷窗草稿")
            })
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
        let file = data.0.join(settings::FILE_NAME);
        let root = workspace.0.to_str().unwrap();

        for newer in [false, true] {
            let mut state = WorkspaceState::default();
            let opened = state.activate(&file, root, "", true).unwrap();
            let session = opened.session;
            let base_revision = opened.draft.revision;
            let storage = state.workspace(&session).unwrap();
            let revision = base_revision + 1;
            let request_id = format!("request-{revision}");
            let record = storage
                .with_storage(|storage| {
                    storage.save_draft(CAPTURE_DRAFT, revision, "合成草稿")?;
                    let record =
                        storage.save_record(&request_id, CAPTURE_DRAFT, revision, "合成草稿")?;
                    if newer {
                        storage.save_draft(CAPTURE_DRAFT, revision + 1, "后来编辑")?;
                    }
                    Ok::<_, StorageError>(record)
                })
                .unwrap();
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
                    .with_storage(|storage| storage.save_record(
                        &request_id,
                        CAPTURE_DRAFT,
                        revision,
                        "合成草稿"
                    ))
                    .unwrap(),
                record
            );
        }
    }

    #[test]
    fn resume_normalizes_a_legacy_empty_capture_directory() {
        let workspace = TestDir::new();
        let data = TestDir::new();
        let file = data.0.join(settings::FILE_NAME);
        let legacy = serde_json::json!({
            "lastWorkspaceRoot": workspace.0.to_str().unwrap(),
            "lastCaptureDir": "",
        });
        fs::write(&file, legacy.to_string()).unwrap();

        let opened = WorkspaceState::default().resume(&file).unwrap().unwrap();
        assert_eq!(opened.capture_dir, "notes");
        assert!(workspace.0.join("notes").is_dir());
        assert_eq!(settings::read(&file).unwrap().last_capture_dir, "notes");
    }

    #[test]
    fn updates_workspace_dirs_without_reopening_the_session() {
        let workspace = TestDir::new();
        let data = TestDir::new();
        let file = data.0.join(settings::FILE_NAME);
        let root = workspace.0.to_str().unwrap();
        let mut state = WorkspaceState::default();
        let opened = state.activate(&file, root, "", true).unwrap();
        let session = opened.session;

        assert_eq!(
            state
                .update_dirs(&file, &session, " \t", None)
                .unwrap_err()
                .code,
            "INVALID_INPUT",
        );
        let updated = state
            .update_dirs(&file, &session, "sources", Some("journal/daily"))
            .unwrap();
        assert_eq!(updated.session, session);
        assert_eq!(updated.raw_dir, "sources");
        assert_eq!(updated.capture_dir, "journal/daily");
        assert!(workspace.0.join("sources").is_dir());
        assert!(workspace.0.join("journal/daily").is_dir());
        assert_eq!(
            settings::read(&file).unwrap().last_capture_dir,
            "journal/daily"
        );

        state
            .update_dirs(&file, &session, "library/sources", Some(" \t"))
            .unwrap();
        let capture = state.load_capture().unwrap();
        assert_eq!(capture.session, session);
        assert_eq!(capture.raw_dir, "library/sources");
        assert_eq!(capture.capture_dir, "journal/daily");

        let resumed = WorkspaceState::default().resume(&file).unwrap().unwrap();
        assert_eq!(resumed.raw_dir, capture.raw_dir);
        assert_eq!(resumed.capture_dir, capture.capture_dir);
    }

    #[test]
    fn switching_revokes_old_sessions_and_resume_accepts_a_restored_directory() {
        let first = TestDir::new();
        let second = TestDir::new();
        let data = TestDir::new();
        let file = data.0.join(settings::FILE_NAME);
        let first_workspace = first.0.join("workspace");
        fs::create_dir(&first_workspace).unwrap();
        let first_root = first_workspace.to_str().unwrap();
        let second_root = second.0.to_str().unwrap();
        let mut state = WorkspaceState::default();
        assert!(state.resume(&file).unwrap().is_none());
        let first_session = state.activate(&file, first_root, "", true).unwrap().session;
        let refreshed_session = state.activate(&file, first_root, "", true).unwrap().session;
        assert!(state.workspace(&first_session).is_err());
        assert!(
            state
                .activate(&file, second_root, "../outside", true)
                .is_err()
        );
        assert!(state.workspace(&refreshed_session).is_ok());
        let second_session = state
            .activate(&file, second_root, "", true)
            .unwrap()
            .session;
        let latest_session = state.activate(&file, first_root, "", true).unwrap().session;
        assert!(state.workspace(&refreshed_session).is_err());
        assert!(state.workspace(&second_session).is_err());
        assert!(state.workspace(&latest_session).is_ok());
        let moved_root = first.0.join("backup");
        fs::rename(&first_workspace, &moved_root).unwrap();
        let mut state = WorkspaceState::default();
        assert!(state.resume(&file).is_err());
        fs::create_dir(&first_workspace).unwrap();
        let resumed = state.resume(&file).unwrap().unwrap();
        assert_eq!(resumed.root, first_root);
    }
}
