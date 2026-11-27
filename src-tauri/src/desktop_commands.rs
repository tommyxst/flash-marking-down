use std::path::Path;
use tauri::{WebviewWindow, async_runtime};
use tauri_plugin_dialog::DialogExt;

use crate::capture_commands::with_workspace;
use crate::dto::AppError;
use crate::platform::{display_local_path, open_file_default, reveal_file_in_explorer};
use crate::storage::{is_in_capture_dir, physical_path, wiki::CAPTURE_TARGET_MESSAGE};
use crate::workspace::{capture_picker_path, picked_capture_directory, workspace_root};

const INVALID_ROOT: &str = "请选择可访问的工作区文件夹";
const INVALID_CAPTURE: &str =
    "请选择工作区内的独立子文件夹存放笔记（不能直接选择根目录或应用状态目录）";
const INVALID_RAW: &str =
    "请选择工作区内的独立子文件夹作为原始文件目录（不能直接选择根目录或应用状态目录）";
const INVALID_MARKDOWN: &str =
    "请选择工作区内可访问的 UTF-8 Markdown 文件（不可选原始随手记或状态目录）";

#[tauri::command]
pub(crate) async fn open_external_url(url: String) -> Result<(), AppError> {
    async_runtime::spawn_blocking(move || crate::platform::open_url_default(&url))
        .await
        .map_err(|_| AppError::from_message("CANCELLED", "url_open", true, "打开网页操作中断"))?
}

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
    let (title, subdir_error) = match kind.as_str() {
        "root" => ("选择工作区目录", None),
        "capture" => ("选择保存目录", Some(INVALID_CAPTURE)),
        "raw" => ("选择原始文件目录（raw）", Some(INVALID_RAW)),
        _ => return Err(AppError::invalid("workspace", "无效的目录选择类型")),
    };

    async_runtime::spawn_blocking(move || {
        let root_path = Path::new(&root);
        let checked_root = workspace_root(root_path);
        let start = match (subdir_error.is_some(), &checked_root) {
            (false, Ok(root)) => Some(root.clone()),
            (false, Err(_)) => None,
            (true, Ok(root)) => Some(
                capture_picker_path(root, Path::new(&capture_dir)).unwrap_or_else(|_| root.clone()),
            ),
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
            .ok_or_else(|| AppError::invalid("workspace", subdir_error.unwrap_or(INVALID_ROOT)))?;
        if let Some(message) = subdir_error {
            let checked_root = checked_root.expect("subdirectory requires valid root");
            return picked_capture_directory(&checked_root, &capture_dir, path)
                .map(Some)
                .map_err(|e| {
                    AppError::new(
                        "OUT_OF_SCOPE",
                        "workspace",
                        false,
                        message,
                        Some(e.to_string()),
                    )
                });
        }

        let selected_root = workspace_root(path).map_err(|e| {
            AppError::new(
                "OUT_OF_SCOPE",
                "workspace",
                false,
                INVALID_ROOT,
                Some(e.to_string()),
            )
        })?;
        if checked_root.is_ok_and(|root| root == selected_root) {
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
            .markdown_full_path(physical_path(&relative_path))
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
    let (files, capture_dir, captured) = with_workspace(window.clone(), session, |storage| {
        Ok((
            storage.workspace_files(),
            storage.capture_dir().to_owned(),
            storage.capture_paths()?,
        ))
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
        let (key, full) = files.picked_markdown(path).map_err(|_| invalid())?;
        if is_in_capture_dir(&key, &capture_dir) || captured.contains(&key) {
            return Err(AppError::invalid("wiki", CAPTURE_TARGET_MESSAGE));
        }
        std::fs::read_to_string(full).map_err(|_| invalid())?;
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
    let (files, capture_dir, captured) = with_workspace(window, session, |storage| {
        Ok((
            storage.workspace_files(),
            storage.capture_dir().to_owned(),
            storage.capture_paths()?,
        ))
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
            .filter(|path| !is_in_capture_dir(path, &capture_dir) && !captured.contains(path))
            .collect();
        result.sort();
        Ok(result)
    })
    .await
    .map_err(|_| AppError::from_message("CANCELLED", "wiki", true, "扫描工作区文件中断"))?
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) & 0x3f] as char);
        out.push(TABLE[(n >> 12) & 0x3f] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

#[tauri::command]
pub(crate) async fn read_workspace_image(
    window: WebviewWindow,
    session: String,
    base_path: Option<String>,
    image_path: String,
) -> Result<String, AppError> {
    let files = with_workspace(window, session, |storage| Ok(storage.workspace_files())).await?;
    async_runtime::spawn_blocking(move || {
        let (mime, bytes) = files
            .read_local_image(base_path.as_deref(), &image_path)
            .map_err(|error| {
                AppError::new(
                    "SOURCE_UNAVAILABLE",
                    "image",
                    false,
                    "无法读取工作区本地图片",
                    Some(error.to_string()),
                )
            })?;
        Ok(format!("data:{mime};base64,{}", encode_base64(&bytes)))
    })
    .await
    .map_err(|_| AppError::from_message("CANCELLED", "image", true, "读取图片已中断"))?
}
