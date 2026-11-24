use std::collections::HashSet;
use std::path::Path;
use tauri::{WebviewWindow, async_runtime};
use tauri_plugin_dialog::DialogExt;

use crate::capture_commands::with_workspace;
use crate::dto::AppError;
use crate::platform::{display_local_path, open_file_default, reveal_file_in_explorer};
use crate::workspace::{capture_picker_path, picked_capture_directory, pin_workspace_root};

const INVALID_ROOT: &str = "请选择本机磁盘上可访问的普通文件夹";
const INVALID_CAPTURE: &str = "请选择工作区内可访问的普通文件夹，应用状态目录不可用";
const INVALID_MARKDOWN: &str =
    "请选择工作区内可访问的 UTF-8 Markdown 文件（不可选原始随手记或状态目录）";

#[tauri::command]
pub(crate) async fn pick_workspace_folder(
    window: WebviewWindow,
    kind: String,
    root: String,
    capture_dir: String,
) -> Result<Option<String>, AppError> {
    if window.label() != "main" {
        return Err(AppError::invalid("workspace", "请在主窗口选择工作区目录"));
    }
    let (capture, title) = match kind.as_str() {
        "root" => (false, "选择工作区目录"),
        "capture" => (true, "选择保存目录"),
        _ => return Err(AppError::invalid("workspace", "无效的目录选择类型")),
    };

    async_runtime::spawn_blocking(move || {
        let root_path = Path::new(&root);
        let start = match (capture, pin_workspace_root(root_path)) {
            (false, Ok((root, _))) => Some(root),
            (false, Err(_)) => None,
            (true, Ok((root, _pins))) => {
                Some(capture_picker_path(&root, Path::new(&capture_dir)).unwrap_or(root))
            }
            (true, Err(_)) => {
                return Err(AppError::invalid("workspace", "请先选择有效的工作区目录"));
            }
        };
        let mut dialog = window.dialog().file().set_parent(&window).set_title(title);
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        let Some(selected) = dialog.blocking_pick_folder() else {
            return Ok(None);
        };
        let path = selected
            .as_path()
            .ok_or_else(|| AppError::invalid("workspace", INVALID_ROOT))?;
        if capture {
            return picked_capture_directory(root_path, &capture_dir, path)
                .map(Some)
                .map_err(|e| {
                    AppError::new(
                        "OUT_OF_SCOPE",
                        "workspace",
                        false,
                        INVALID_CAPTURE,
                        Some(e.to_string()),
                    )
                });
        }

        let (selected_root, _pins) = pin_workspace_root(path).map_err(|e| {
            AppError::new(
                "OUT_OF_SCOPE",
                "workspace",
                false,
                INVALID_ROOT,
                Some(e.to_string()),
            )
        })?;
        if pin_workspace_root(root_path).is_ok_and(|(root, _)| root == selected_root) {
            return Ok(Some(root));
        }
        Ok(Some(
            path.to_str()
                .ok_or_else(|| AppError::invalid("workspace", INVALID_ROOT))?
                .to_owned(),
        ))
    })
    .await
    .map_err(|_| AppError::from_message("CANCELLED", "workspace", true, "目录选择已中断，请重试"))?
}

#[tauri::command]
pub(crate) async fn desktop_window_action(
    window: WebviewWindow,
    action: String,
) -> Result<(), AppError> {
    match (window.label(), action.as_str()) {
        ("main" | "capture", "drag") => window.start_dragging(),
        ("main", "minimize") => window.minimize(),
        ("main", "maximize") => window.is_maximized().and_then(|maximized| {
            if maximized {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }),
        ("main", "close") => window.close(),
        _ => return Err(AppError::invalid("window", "此窗口不支持该操作")),
    }
    .map_err(|error| {
        eprintln!("窗口操作失败（{error}）");
        AppError::new(
            "WINDOW_FAILED",
            "window",
            true,
            "窗口操作失败，请重试",
            Some(error.to_string()),
        )
    })
}

#[tauri::command]
pub(crate) async fn is_main_maximized(window: WebviewWindow) -> Result<bool, AppError> {
    if window.label() != "main" {
        return Err(AppError::invalid("window", "此窗口不支持该操作"));
    }
    window.is_maximized().map_err(|error| {
        AppError::new(
            "WINDOW_FAILED",
            "window",
            true,
            "无法读取窗口状态",
            Some(error.to_string()),
        )
    })
}

#[tauri::command]
pub(crate) async fn open_workspace_file(
    window: WebviewWindow,
    session: String,
    relative_path: String,
    mode: String,
) -> Result<(), AppError> {
    if !matches!(mode.as_str(), "open" | "reveal") {
        return Err(AppError::invalid("file_open", "无效的打开方式"));
    }
    let full = with_workspace(window, session, move |storage| {
        storage
            .workspace_files()
            .markdown_full_path(&relative_path)
            .map_err(|error| {
                AppError::new(
                    "SOURCE_UNAVAILABLE",
                    "file_open",
                    false,
                    "找不到可打开的 Markdown 文件，或文件不在工作区范围内",
                    Some(error.to_string()),
                )
            })
    })
    .await?;
    async_runtime::spawn_blocking(move || {
        if mode == "reveal" {
            reveal_file_in_explorer(&full).map_err(|detail| {
                AppError::new(
                    "OPERATION_FAILED",
                    "file_open",
                    true,
                    "无法在文件资源管理器中定位该文件",
                    Some(detail),
                )
            })
        } else {
            open_file_default(&full)
                .map_err(|msg| AppError::from_message("OPERATION_FAILED", "file_open", true, msg))
        }
    })
    .await
    .map_err(|_| AppError::from_message("CANCELLED", "file_open", true, "打开文件操作中断"))?
}

#[tauri::command]
pub(crate) async fn pick_workspace_markdown(
    window: WebviewWindow,
    session: String,
    kind: String,
) -> Result<Option<String>, AppError> {
    let title = match kind.as_str() {
        "wiki" => "选择 Wiki Markdown 文件",
        "tasks" => "选择任务清单 Markdown 文件",
        _ => return Err(AppError::invalid("wiki", "无效的文件用途")),
    };
    let (files, capture_names) = with_workspace(window.clone(), session, |storage| {
        let names = storage
            .capture_file_names()?
            .into_iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        Ok((storage.workspace_files(), names))
    })
    .await?;

    async_runtime::spawn_blocking(move || {
        let start = Path::new(&display_local_path(files.root_path())).to_path_buf();
        let Some(selected) = window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(title)
            .add_filter("Markdown", &["md"])
            .set_directory(start)
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let invalid = || AppError::invalid("wiki", INVALID_MARKDOWN);
        let path = selected.as_path().ok_or_else(invalid)?;
        let canonical = path.canonicalize().map_err(|_| invalid())?;
        let relative = canonical
            .strip_prefix(files.root_path())
            .map_err(|_| invalid())?
            .to_str()
            .ok_or_else(invalid)?
            .replace('\\', "/");
        let key = files.markdown_key(&relative).map_err(|_| invalid())?;
        let file_name = key.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
        if capture_names.contains(&file_name) {
            return Err(AppError::invalid("wiki", "原始随手记文件不能作为写入目标"));
        }
        let bytes = files.read_markdown(&key).map_err(|_| invalid())?;
        std::str::from_utf8(&bytes).map_err(|_| invalid())?;
        Ok(Some(key))
    })
    .await
    .map_err(|_| AppError::from_message("CANCELLED", "wiki", true, "文件选择已中断，请重试"))?
}

#[tauri::command]
pub(crate) async fn list_workspace_markdown_files(
    window: WebviewWindow,
    session: String,
) -> Result<Vec<String>, AppError> {
    let (files, capture_names) = with_workspace(window, session, |storage| {
        let names = storage
            .capture_file_names()?
            .into_iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        Ok((storage.workspace_files(), names))
    })
    .await?;
    async_runtime::spawn_blocking(move || {
        let (paths, _) = files.markdown_paths().map_err(|error| {
            AppError::new(
                "SOURCE_UNAVAILABLE",
                "wiki",
                true,
                "无法扫描工作区 Markdown 文件",
                Some(error.to_string()),
            )
        })?;
        let mut result: Vec<String> = paths
            .into_iter()
            .filter(|path| {
                let name = path.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
                !capture_names.contains(&name)
            })
            .collect();
        result.sort();
        Ok(result)
    })
    .await
    .map_err(|_| AppError::from_message("CANCELLED", "wiki", true, "扫描工作区文件中断"))?
}
