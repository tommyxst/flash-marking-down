use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OpenFlags, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::dto::AppError;
use crate::storage::{StorageError, error};
use crate::workspace::atomic_write;

pub(crate) const FILE_NAME: &str = "settings.json";
const APP_ID: &str = "app.wikra.desktop";
const LEGACY_APP_ID: &str = "com.flash-marking-down.desktop";
static SETTINGS_LOCK: Mutex<()> = Mutex::new(());

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Theme {
    #[default]
    Slate,
    Paper,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DocWidth {
    #[default]
    Readable,
    Full,
}

fn default_true() -> bool {
    true
}

fn default_buffer_seconds() -> u32 {
    60
}

fn deserialize_buffer_seconds<'de, D: serde::Deserializer<'de>>(input: D) -> Result<u32, D::Error> {
    i64::deserialize(input).map(|value| value.clamp(0, 300) as u32)
}

#[derive(Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AppSettings {
    version: u32,
    pub(crate) last_workspace_root: Option<String>,
    pub(crate) last_capture_dir: String,
    pub(crate) hotkey: String,
    pub(crate) auto_organize: bool,
    pub(crate) auto_sync_memory: bool,
    #[serde(default)]
    theme: Theme,
    #[serde(default)]
    doc_width: DocWidth,
    #[serde(default = "default_true")]
    pub(crate) allow_note_fulltext_to_model: bool,
    #[serde(default = "default_true")]
    pub(crate) quiet_organize: bool,
    #[serde(
        default = "default_buffer_seconds",
        deserialize_with = "deserialize_buffer_seconds"
    )]
    pub(crate) buffer_seconds: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            last_workspace_root: None,
            last_capture_dir: crate::workspace::DEFAULT_CAPTURE_DIR.into(),
            hotkey: "win-alt-n".into(),
            auto_organize: true,
            auto_sync_memory: true,
            theme: Theme::default(),
            doc_width: DocWidth::default(),
            allow_note_fulltext_to_model: default_true(),
            quiet_organize: default_true(),
            buffer_seconds: default_buffer_seconds(),
        }
    }
}

pub(crate) fn settings_path(app: &AppHandle) -> Result<PathBuf, StorageError> {
    app.path()
        .app_local_data_dir()
        .map(|dir| dir.join(FILE_NAME))
        .map_err(settings_error)
}

fn settings_error(detail: impl std::fmt::Display) -> StorageError {
    error(
        "SETTINGS_FAILED",
        "settings",
        None,
        true,
        "无法读取或保存应用设置",
        detail,
    )
}

fn legacy_row<T>(
    path: PathBuf,
    table: &str,
    query: &str,
    read: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
) -> io::Result<Option<T>> {
    if !path.try_exists()? {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(io::Error::other)?;
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = ?1)",
            [table],
            |r| r.get(0),
        )
        .map_err(io::Error::other)?;
    if !exists {
        return Ok(None);
    }
    connection
        .query_row(query, [], read)
        .optional()
        .map_err(io::Error::other)
}

fn import_settings(dir: &Path) -> io::Result<AppSettings> {
    if dir.file_name().is_some_and(|name| name == APP_ID)
        && let Some(parent) = dir.parent()
        && !dir.join("last-workspace.sqlite").try_exists()?
        && !dir.join("quick-entry-hotkey.sqlite").try_exists()?
    {
        let legacy = parent.join(LEGACY_APP_ID);
        match fs::read(legacy.join(FILE_NAME)) {
            Ok(data) => return Ok(serde_json::from_slice(&data)?),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return import_settings(&legacy),
            Err(err) => return Err(err),
        }
    }
    let mut settings = AppSettings::default();
    if let Some((root, capture_dir)) = legacy_row(
        dir.join("last-workspace.sqlite"),
        "last_workspace",
        "SELECT root, capture_dir FROM last_workspace WHERE singleton = 1",
        |row| Ok((row.get(0)?, row.get(1)?)),
    )? {
        settings.last_workspace_root = Some(root);
        settings.last_capture_dir = capture_dir;
    }
    if let Some(binding) = legacy_row(
        dir.join("quick-entry-hotkey.sqlite"),
        "capture_hotkey",
        "SELECT binding FROM capture_hotkey WHERE id = 1",
        |row| row.get(0),
    )? {
        settings.hotkey = binding;
    }
    Ok(settings)
}

fn access(path: &Path, change: Option<impl FnOnce(&mut AppSettings)>) -> io::Result<AppSettings> {
    let _lock = SETTINGS_LOCK
        .lock()
        .map_err(|_| io::Error::other("应用设置不可用"))?;
    let dir = path
        .parent()
        .expect("settings file is in the application data directory");
    let (mut settings, first_use) = match fs::read(path) {
        Ok(data) => (serde_json::from_slice(&data)?, false),
        Err(err) if err.kind() == io::ErrorKind::NotFound => (import_settings(dir)?, true),
        Err(err) => return Err(err),
    };
    if settings.version != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "不支持该应用设置版本",
        ));
    }
    let write = first_use || change.is_some();
    if let Some(change) = change {
        change(&mut settings);
    }
    settings.buffer_seconds = settings.buffer_seconds.min(300);
    if write {
        fs::create_dir_all(dir)?;
        atomic_write(path, &serde_json::to_vec_pretty(&settings)?)?;
    }
    Ok(settings)
}

pub(crate) fn read(path: &Path) -> Result<AppSettings, StorageError> {
    access(path, None::<fn(&mut AppSettings)>).map_err(settings_error)
}

pub(crate) fn update(
    path: &Path,
    change: impl FnOnce(&mut AppSettings),
) -> Result<AppSettings, StorageError> {
    access(path, Some(change)).map_err(settings_error)
}

fn main_settings_path(window: &WebviewWindow) -> Result<PathBuf, AppError> {
    if window.label() != "main" {
        return Err(AppError::invalid(
            "settings",
            "请在主窗口查看或修改应用设置",
        ));
    }
    Ok(settings_path(window.app_handle())?)
}

#[tauri::command]
pub(crate) async fn get_app_settings(window: WebviewWindow) -> Result<AppSettings, AppError> {
    Ok(read(&main_settings_path(&window)?)?)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn save_app_settings(
    window: WebviewWindow,
    auto_organize: Option<bool>,
    auto_sync_memory: Option<bool>,
    theme: Option<String>,
    doc_width: Option<String>,
    allow_note_fulltext_to_model: Option<bool>,
    quiet_organize: Option<bool>,
    buffer_seconds: Option<i64>,
) -> Result<AppSettings, AppError> {
    let theme = match theme.as_deref() {
        Some("slate") => Some(Theme::Slate),
        Some("paper") => Some(Theme::Paper),
        None => None,
        _ => return Err(AppError::invalid("settings", "主题必须为 slate 或 paper")),
    };
    let doc_width = match doc_width.as_deref() {
        Some("readable") => Some(DocWidth::Readable),
        Some("full") => Some(DocWidth::Full),
        None => None,
        _ => {
            return Err(AppError::invalid(
                "settings",
                "文档宽度必须为 readable 或 full",
            ));
        }
    };
    let updated = update(&main_settings_path(&window)?, |settings| {
        if let Some(value) = auto_organize {
            settings.auto_organize = value;
        }
        if let Some(value) = auto_sync_memory {
            settings.auto_sync_memory = value;
        }
        if let Some(value) = theme {
            settings.theme = value;
        }
        if let Some(value) = doc_width {
            settings.doc_width = value;
        }
        if let Some(value) = allow_note_fulltext_to_model {
            settings.allow_note_fulltext_to_model = value;
        }
        if let Some(value) = quiet_organize {
            settings.quiet_organize = value;
        }
        if let Some(value) = buffer_seconds {
            settings.buffer_seconds = value.clamp(0, 300) as u32;
        }
    })?;
    if auto_sync_memory == Some(true) {
        crate::m2_commands::wake_memory_backfill(window.app_handle());
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::TestDir;

    #[test]
    fn imports_existing_selection_and_hotkey_once_then_merges_preference_changes() {
        let dir = TestDir::new();
        let old_selection = Connection::open(dir.0.join("last-workspace.sqlite")).unwrap();
        old_selection.execute_batch(
            "CREATE TABLE last_workspace (singleton INTEGER, root TEXT, capture_dir TEXT, workspace_id TEXT, file_identity TEXT);
             INSERT INTO last_workspace VALUES (1, 'synthetic-root', 'inbox', 'old-id', 'old-volume');"
        ).unwrap();
        let old_hotkey = Connection::open(dir.0.join("quick-entry-hotkey.sqlite")).unwrap();
        old_hotkey
            .execute_batch(
                "CREATE TABLE capture_hotkey (id INTEGER, binding TEXT);
             INSERT INTO capture_hotkey VALUES (1, 'ctrl-alt-n');",
            )
            .unwrap();

        let path = dir.0.join(FILE_NAME);
        let initial = read(&path).unwrap();
        assert_eq!(
            initial.last_workspace_root.as_deref(),
            Some("synthetic-root")
        );
        assert_eq!(initial.last_capture_dir, "inbox");
        assert_eq!(initial.hotkey, "ctrl-alt-n");
        assert!(initial.auto_organize && initial.auto_sync_memory);
        old_hotkey
            .execute("UPDATE capture_hotkey SET binding = 'ctrl-alt-m'", [])
            .unwrap();
        std::thread::scope(|threads| {
            threads.spawn(|| update(&path, |s| s.auto_organize = false).unwrap());
            threads.spawn(|| update(&path, |s| s.auto_sync_memory = false).unwrap());
        });
        let final_settings = read(&path).unwrap();
        assert_eq!(final_settings.hotkey, "ctrl-alt-n");
        assert!(!final_settings.auto_organize && !final_settings.auto_sync_memory);
        assert_eq!(
            final_settings.last_workspace_root,
            initial.last_workspace_root
        );

        let partial = dir.0.join("partial-settings.json");
        fs::write(&partial, r#"{"lastWorkspaceRoot":"synthetic-root"}"#).unwrap();
        let loaded = read(&partial).unwrap();
        assert_eq!(AppSettings::default().last_capture_dir, "notes");
        assert_eq!(loaded.version, 1);
        assert_eq!(
            loaded.last_workspace_root.as_deref(),
            Some("synthetic-root")
        );
        assert_eq!(loaded.last_capture_dir, "notes");
        assert_eq!(loaded.hotkey, "win-alt-n");
        assert!(loaded.auto_organize && loaded.auto_sync_memory);
        let json = serde_json::to_value(&loaded).unwrap();
        assert_eq!(json["theme"], "slate");
        assert_eq!(json["docWidth"], "readable");
        assert_eq!(json["allowNoteFulltextToModel"], true);
        assert_eq!(json["quietOrganize"], true);
        assert_eq!(json["bufferSeconds"], 60);

        let invalid_ver = dir.0.join("invalid-ver.json");
        fs::write(&invalid_ver, r#"{"version":2}"#).unwrap();
        assert!(read(&invalid_ver).is_err());
    }

    #[test]
    fn brand_change_imports_old_config_and_keeps_new_preferences() {
        let dir = TestDir::new();
        let legacy = dir.0.join(LEGACY_APP_ID);
        fs::create_dir(&legacy).unwrap();
        fs::write(
            legacy.join(FILE_NAME),
            r#"{"version":1,"lastWorkspaceRoot":"old-root","bufferSeconds":-1}"#,
        )
        .unwrap();
        let path = dir.0.join(APP_ID).join(FILE_NAME);
        let imported = read(&path).unwrap();
        assert_eq!(imported.last_workspace_root.as_deref(), Some("old-root"));
        assert_eq!(imported.buffer_seconds, 0);
        update(&path, |saved| {
            saved.theme = Theme::Paper;
            saved.doc_width = DocWidth::Full;
            saved.buffer_seconds = 600;
            saved.allow_note_fulltext_to_model = false;
            saved.quiet_organize = false;
        })
        .unwrap();
        let json = serde_json::to_value(read(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["theme"], "paper");
        assert_eq!(json["docWidth"], "full");
        assert_eq!(json["bufferSeconds"], 300);
        assert_eq!(json["allowNoteFulltextToModel"], false);
        assert_eq!(json["quietOrganize"], false);
        assert_eq!(json["lastWorkspaceRoot"], "old-root");
    }
}
