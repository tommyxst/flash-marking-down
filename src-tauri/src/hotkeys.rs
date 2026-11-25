use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use tauri::async_runtime::{Receiver, Sender, channel};
use tauri::{AppHandle, Manager, WebviewWindow};

use crate::dto::{AppError, HotkeyChoiceDto, HotkeyStatusDto};

use crate::platform::hotkey::{
    Binding, MOD_ALT, MOD_CONTROL, MOD_WIN, Registration, SETTINGS_MESSAGE, STOP_MESSAGE,
    WM_HOTKEY, init_message_queue, next_message, post,
};
use crate::settings;

const SERVICE_UNAVAILABLE: &str = "全局快捷键服务不可用，请重启应用";

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
            settings::read(path)
                .map_err(|error| error.safe_message.to_owned())
                .and_then(|settings| find_binding(&settings.hotkey).map_err(str::to_owned))
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
        settings::update(path, |settings| settings.hotkey = binding.id.to_owned())
            .map_err(|error| format!("无法保存快捷键设置（{}）", error.technical_detail))?;
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

fn run_worker(
    app: AppHandle,
    incoming: mpsc::Receiver<Request>,
    dispatch: Arc<Mutex<()>>,
    started: mpsc::SyncSender<u32>,
) {
    if started.send(init_message_queue()).is_err() {
        return;
    }
    let path = settings::settings_path(&app).map_err(|error| error.safe_message.to_owned());
    let mut state = HotkeyState::load(path);
    loop {
        let (message, wparam) = match next_message() {
            Ok(Some(event)) => event,
            Ok(None) => break,
            Err(error) => {
                eprintln!("无法接收全局快捷键事件（{error}）");
                break;
            }
        };
        match message {
            STOP_MESSAGE => break,
            WM_HOTKEY
                if state
                    .registration
                    .as_ref()
                    .is_some_and(|registered| registered.0.registration_id as usize == wparam) =>
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
        let path = dir.0.join(settings::FILE_NAME);
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
        assert_eq!(settings::read(&path).unwrap().hotkey, PREVIOUS.id);

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
        assert_eq!(settings::read(&path).unwrap().hotkey, PREVIOUS.id);
        assert!(Registration::register(&NEXT).is_ok());

        state.path = Ok(path.clone());
        state.select(&NEXT).unwrap();
        assert_eq!(state.view().id, NEXT.id);
        assert_eq!(settings::read(&path).unwrap().hotkey, NEXT.id);
        assert!(Registration::register(&PREVIOUS).is_ok());
    }
}
