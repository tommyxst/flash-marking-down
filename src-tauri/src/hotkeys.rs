use std::ffi::c_void;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use rusqlite::{Connection, OpenFlags};
use tauri::async_runtime::{Receiver, Sender, channel};
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::dto::{AppError, HotkeyChoiceDto, HotkeyStatusDto};

const WM_HOTKEY: u32 = 0x0312;
const WM_USER: u32 = 0x0400;
const WM_APP: u32 = 0x8000;
const SETTINGS_MESSAGE: u32 = WM_APP;
const STOP_MESSAGE: u32 = WM_APP + 1;
const PM_NOREMOVE: u32 = 0;
const MOD_ALT: u32 = 0x0001;
const MOD_CONTROL: u32 = 0x0002;
const MOD_WIN: u32 = 0x0008;
const MOD_NOREPEAT: u32 = 0x4000;
const ERROR_HOTKEY_ALREADY_REGISTERED: i32 = 1409;
const PREFERENCES_FILE: &str = "quick-entry-hotkey.sqlite";
const SERVICE_UNAVAILABLE: &str = "全局快捷键服务不可用，请重启应用";

#[repr(C)]
struct Message {
    window: *mut c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point: [i32; 2],
    private: u32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterHotKey(window: *mut c_void, id: i32, modifiers: u32, key: u32) -> i32;
    fn UnregisterHotKey(window: *mut c_void, id: i32) -> i32;
    fn PeekMessageW(
        message: *mut Message,
        window: *mut c_void,
        min: u32,
        max: u32,
        mode: u32,
    ) -> i32;
    fn GetMessageW(message: *mut Message, window: *mut c_void, min: u32, max: u32) -> i32;
    fn PostThreadMessageW(thread: u32, message: u32, wparam: usize, lparam: isize) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

#[derive(Clone, Copy)]
struct Binding {
    id: &'static str,
    label: &'static str,
    registration_id: i32,
    modifiers: u32,
    key: u32,
}

static BINDINGS: [Binding; 3] = [
    Binding {
        id: "win-alt-n",
        label: "Win+Alt+N",
        registration_id: 1,
        modifiers: MOD_WIN | MOD_ALT,
        key: b'N' as u32,
    },
    Binding {
        id: "ctrl-alt-n",
        label: "Ctrl+Alt+N",
        registration_id: 2,
        modifiers: MOD_CONTROL | MOD_ALT,
        key: b'N' as u32,
    },
    Binding {
        id: "ctrl-alt-m",
        label: "Ctrl+Alt+M",
        registration_id: 3,
        modifiers: MOD_CONTROL | MOD_ALT,
        key: b'M' as u32,
    },
];
const DEFAULT_BINDING: &Binding = &BINDINGS[0];

type PreferenceError = Box<dyn std::error::Error>;

struct Registration(&'static Binding);

impl Registration {
    fn register(binding: &'static Binding) -> Result<Self, String> {
        if unsafe {
            RegisterHotKey(
                null_mut(),
                binding.registration_id,
                binding.modifiers | MOD_NOREPEAT,
                binding.key,
            )
        } != 0
        {
            return Ok(Self(binding));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_HOTKEY_ALREADY_REGISTERED) {
            Err(format!("{} 已被 Windows 或其他应用占用", binding.label))
        } else {
            Err(format!("无法注册快捷键 {}（{error}）", binding.label))
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if unsafe { UnregisterHotKey(null_mut(), self.0.registration_id) } == 0 {
            let error = io::Error::last_os_error();
            eprintln!("无法注销快捷键 {}（{error}）", self.0.label);
        }
    }
}

struct HotkeyState {
    path: Result<PathBuf, String>,
    selected: &'static Binding,
    registration: Option<Registration>,
    error: Option<String>,
}

impl HotkeyState {
    fn load(path: Result<PathBuf, String>) -> Self {
        let mut state = Self {
            path,
            selected: DEFAULT_BINDING,
            registration: None,
            error: None,
        };
        let selected = state.path.as_ref().map_err(Clone::clone).and_then(|path| {
            load_preference(path).map_err(|error| format!("无法读取快捷键设置（{error}）"))
        });
        state.error = selected
            .and_then(|binding| {
                state.selected = binding;
                state.registration = Some(Registration::register(binding)?);
                Ok(())
            })
            .err();
        state
    }

    fn select(&mut self, binding: &'static Binding) -> Result<(), String> {
        let path = self.path.as_ref().map_err(Clone::clone)?;
        let changed = self.selected.id != binding.id || self.registration.is_none();
        // Keep the old registration until the new binding is recorded for restart.
        let pending = changed
            .then(|| Registration::register(binding))
            .transpose()?;
        save_preference(path, binding).map_err(|error| format!("无法保存快捷键设置（{error}）"))?;
        if let Some(registration) = pending {
            self.selected = binding;
            self.registration = Some(registration);
        }
        Ok(())
    }

    fn view(&self) -> HotkeyStatusDto {
        HotkeyStatusDto {
            id: self.selected.id.to_owned(),
            active: self.registration.is_some(),
            error: self.error.clone(),
            choices: BINDINGS
                .iter()
                .map(|binding| HotkeyChoiceDto {
                    id: binding.id.to_owned(),
                    label: binding.label.to_owned(),
                })
                .collect(),
        }
    }
}

fn load_preference(path: &Path) -> Result<&'static Binding, PreferenceError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(DEFAULT_BINDING),
        Err(error) => return Err(error.into()),
    }
    // Allow SQLite to roll back a hot journal after an interrupted settings write.
    let database = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    database.pragma_update(None, "busy_timeout", 5000)?;
    let initialized: bool = database.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = 'capture_hotkey')",
        [],
        |row| row.get(0),
    )?;
    if !initialized {
        return Ok(DEFAULT_BINDING);
    }
    let binding_id: String = database.query_row(
        "SELECT binding FROM capture_hotkey WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    Ok(find_binding(&binding_id)?)
}

fn save_preference(path: &Path, binding: &Binding) -> Result<(), PreferenceError> {
    fs::create_dir_all(
        path.parent()
            .expect("hotkey settings have a parent directory"),
    )?;
    let mut database = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )?;
    database.pragma_update(None, "busy_timeout", 5000)?;
    let transaction = database.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS capture_hotkey (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            binding TEXT NOT NULL
        ) STRICT;",
    )?;
    transaction.execute(
        "INSERT INTO capture_hotkey(id, binding) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET binding = excluded.binding",
        [binding.id],
    )?;
    transaction.commit()?;
    Ok(())
}

fn find_binding(id: &str) -> Result<&'static Binding, &'static str> {
    BINDINGS
        .iter()
        .find(|binding| binding.id == id)
        .ok_or("不支持该快捷键")
}

struct Request {
    selection: Option<&'static Binding>,
    response: Sender<HotkeyStatusDto>,
}

struct Worker {
    thread: JoinHandle<()>,
    id: u32,
    requests: mpsc::Sender<Request>,
    dispatch: Arc<Mutex<()>>,
}

impl Worker {
    fn start(app: AppHandle) -> Result<Self, String> {
        let (requests, incoming) = mpsc::channel();
        let (started, ready) = mpsc::sync_channel(0);
        let dispatch = Arc::new(Mutex::new(()));
        let receive = dispatch.clone();
        let thread = thread::Builder::new()
            .name("global-hotkey".to_owned())
            .spawn(move || run_worker(app, incoming, receive, started))
            .map_err(|error| format!("无法启动全局快捷键服务（{error}）"))?;
        let id = ready.recv().map_err(|_| SERVICE_UNAVAILABLE.to_owned())?;
        Ok(Self {
            thread,
            id,
            requests,
            dispatch,
        })
    }

    fn request(
        &self,
        selection: Option<&'static Binding>,
    ) -> Result<Receiver<HotkeyStatusDto>, String> {
        let (response, receiver) = channel(1);
        // The worker cannot read the request before the wake succeeds.
        let _dispatch = self
            .dispatch
            .lock()
            .map_err(|_| SERVICE_UNAVAILABLE.to_owned())?;
        post(self.id, SETTINGS_MESSAGE)
            .map_err(|error| format!("{SERVICE_UNAVAILABLE}（{error}）"))?;
        self.requests
            .send(Request {
                selection,
                response,
            })
            .map_err(|_| SERVICE_UNAVAILABLE.to_owned())?;
        Ok(receiver)
    }

    fn stop(self) {
        if let Err(error) = post(self.id, STOP_MESSAGE) {
            eprintln!("无法结束全局快捷键服务（{error}）");
        } else if self.thread.join().is_err() {
            eprintln!("全局快捷键服务意外结束");
        }
    }
}

fn post(thread: u32, message: u32) -> io::Result<()> {
    if unsafe { PostThreadMessageW(thread, message, 0, 0) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn run_worker(
    app: AppHandle,
    incoming: mpsc::Receiver<Request>,
    dispatch: Arc<Mutex<()>>,
    started: mpsc::SyncSender<u32>,
) {
    // MSG can be initialized to zero. PeekMessageW creates the thread's message queue.
    let mut message: Message = unsafe { std::mem::zeroed() };
    unsafe { PeekMessageW(&mut message, null_mut(), WM_USER, WM_USER, PM_NOREMOVE) };
    if started.send(unsafe { GetCurrentThreadId() }).is_err() {
        return;
    }
    let path = app
        .path()
        .app_local_data_dir()
        .map(|dir| dir.join(PREFERENCES_FILE))
        .map_err(|error| format!("无法定位快捷键设置（{error}）"));
    let mut state = HotkeyState::load(path);
    loop {
        match unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } {
            0 => break,
            -1 => {
                eprintln!("无法接收全局快捷键事件（{}）", io::Error::last_os_error());
                break;
            }
            _ => {}
        }
        match message.message {
            STOP_MESSAGE => break,
            WM_HOTKEY
                if state.registration.as_ref().is_some_and(|registered| {
                    registered.0.registration_id as usize == message.wparam
                }) =>
            {
                crate::capture_commands::show_from_resident(&app)
            }
            SETTINGS_MESSAGE => {
                let request = {
                    let Ok(_dispatch) = dispatch.lock() else {
                        break;
                    };
                    incoming.try_recv()
                };
                if let Ok(Request {
                    selection,
                    response,
                }) = request
                {
                    if let Some(binding) = selection {
                        state.error = state.select(binding).err();
                    }
                    let _ = response.try_send(state.view());
                }
            }
            _ => {}
        }
    }
}

struct Hotkeys(Mutex<Result<Worker, String>>);

pub(crate) fn init(app: &AppHandle) {
    app.manage(Hotkeys(Mutex::new(Worker::start(app.clone()))));
}

pub(crate) fn shutdown(app: &AppHandle) {
    let state = app.state::<Hotkeys>();
    let Ok(mut worker) = state.0.lock() else {
        return;
    };
    let previous = std::mem::replace(&mut *worker, Err(SERVICE_UNAVAILABLE.to_owned()));
    drop(worker);
    if let Ok(previous) = previous {
        previous.stop();
    }
}

async fn request(
    window: WebviewWindow,
    selection: Option<&'static Binding>,
) -> Result<HotkeyStatusDto, AppError> {
    if window.label() != "main" {
        return Err(AppError::invalid("hotkey", "请在主窗口设置全局快捷键"));
    }
    let mut response = {
        let state = window.try_state::<Hotkeys>().ok_or_else(|| {
            AppError::from_message("SERVICE_UNAVAILABLE", "hotkey", true, SERVICE_UNAVAILABLE)
        })?;
        let worker = state.0.lock().map_err(|_| {
            AppError::from_message("SERVICE_UNAVAILABLE", "hotkey", true, SERVICE_UNAVAILABLE)
        })?;
        worker
            .as_ref()
            .map_err(|msg| {
                AppError::from_message("SERVICE_UNAVAILABLE", "hotkey", true, msg.clone())
            })?
            .request(selection)
            .map_err(|msg| AppError::from_message("SERVICE_UNAVAILABLE", "hotkey", true, msg))?
    };
    response.recv().await.ok_or_else(|| {
        AppError::from_message("SERVICE_UNAVAILABLE", "hotkey", true, SERVICE_UNAVAILABLE)
    })
}

#[tauri::command]
pub(crate) async fn get_hotkey(window: WebviewWindow) -> Result<HotkeyStatusDto, AppError> {
    request(window, None).await
}

#[tauri::command]
pub(crate) async fn set_hotkey(
    window: WebviewWindow,
    id: String,
) -> Result<HotkeyStatusDto, AppError> {
    let binding = find_binding(&id).map_err(|msg| AppError::invalid("hotkey", msg))?;
    request(window, Some(binding)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::TestDir;

    #[test]
    fn failed_changes_preserve_the_active_and_saved_binding() {
        const MOD_SHIFT: u32 = 0x0004;
        const VK_F22: u32 = 0x85;
        const VK_F23: u32 = 0x86;
        static PREVIOUS: Binding = Binding {
            key: VK_F22,
            modifiers: MOD_CONTROL | MOD_ALT | MOD_SHIFT,
            ..BINDINGS[0]
        };
        static NEXT: Binding = Binding {
            key: VK_F23,
            modifiers: MOD_CONTROL | MOD_ALT | MOD_SHIFT,
            ..BINDINGS[1]
        };
        static OCCUPANT: Binding = Binding {
            registration_id: 4,
            ..NEXT
        };

        let dir = TestDir::new();
        let path = dir.0.join(PREFERENCES_FILE);
        let mut state = HotkeyState {
            path: Ok(path.clone()),
            selected: &PREVIOUS,
            registration: None,
            error: None,
        };
        state.select(&PREVIOUS).unwrap();
        let occupant = Registration::register(&OCCUPANT).unwrap();
        state.error = state.select(&NEXT).err();
        let view = state.view();
        assert_eq!(view.id, PREVIOUS.id);
        assert!(view.active);
        assert_eq!(
            view.error.as_deref(),
            Some("Ctrl+Alt+N 已被 Windows 或其他应用占用")
        );
        assert_eq!(load_preference(&path).unwrap().id, PREVIOUS.id);

        drop(occupant);
        state.path = Ok(dir.0.clone());
        state.error = state.select(&NEXT).err();
        assert!(
            state
                .error
                .as_deref()
                .unwrap()
                .starts_with("无法保存快捷键设置")
        );
        assert_eq!(state.view().id, PREVIOUS.id);
        assert_eq!(load_preference(&path).unwrap().id, PREVIOUS.id);
        assert!(Registration::register(&NEXT).is_ok());

        state.path = Ok(path.clone());
        state.select(&NEXT).unwrap();
        assert_eq!(state.view().id, NEXT.id);
        assert_eq!(load_preference(&path).unwrap().id, NEXT.id);
        assert!(Registration::register(&PREVIOUS).is_ok());
    }
}
