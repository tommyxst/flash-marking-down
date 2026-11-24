#[cfg(windows)]
mod capture_commands;
#[cfg(windows)]
mod credential_commands;
#[cfg(windows)]
mod desktop_commands;
pub mod dto;
#[cfg(windows)]
mod hotkeys;
#[cfg(windows)]
mod m2_commands;
mod model;
mod platform;
#[cfg(windows)]
pub mod storage;
#[cfg(windows)]
mod workspace;

pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(windows)]
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .manage(std::sync::Mutex::new(
            capture_commands::WorkspaceState::default(),
        ))
        .manage(capture_commands::DesktopWindows::default())
        .on_window_event(capture_commands::on_window_event)
        .setup(|app| {
            capture_commands::init_tray(app.handle())?;
            hotkeys::init(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            capture_commands::open_workspace,
            capture_commands::resume_workspace,
            capture_commands::show_capture_window,
            capture_commands::load_capture_workspace,
            capture_commands::close_capture_window,
            capture_commands::list_recent_captures,
            capture_commands::search_records,
            capture_commands::rebuild_search_index,
            capture_commands::open_record_source,
            capture_commands::persist_draft,
            capture_commands::commit_capture,
            capture_commands::confirm_capture,
            desktop_commands::pick_workspace_folder,
            desktop_commands::desktop_window_action,
            desktop_commands::is_main_maximized,
            desktop_commands::open_workspace_file,
            desktop_commands::pick_workspace_markdown,
            desktop_commands::list_workspace_markdown_files,
            credential_commands::list_credentials,
            credential_commands::set_credential,
            credential_commands::save_credential,
            m2_commands::get_model_config,
            m2_commands::set_model_config,
            m2_commands::list_analyses,
            m2_commands::analyze_source,
            m2_commands::prepare_analysis_change,
            m2_commands::prepare_answer,
            m2_commands::cancel_answer_preparation,
            m2_commands::answer_query,
            m2_commands::copy_answer,
            m2_commands::list_disagreements,
            m2_commands::create_disagreement,
            m2_commands::review_disagreement,
            m2_commands::wiki_targets,
            m2_commands::map_wiki_target,
            m2_commands::create_wiki_target,
            m2_commands::wiki_changes,
            m2_commands::review_wiki_change,
            m2_commands::apply_wiki_changes,
            m2_commands::preview_wiki_undo,
            m2_commands::rebase_wiki_change,
            m2_commands::wiki_tasks,
            m2_commands::prepare_task_status,
            hotkeys::get_hotkey,
            hotkeys::set_hotkey,
        ]);

    builder
        .build(tauri::generate_context!())
        .expect("desktop startup failed")
        .run(|app, event| {
            #[cfg(windows)]
            match event {
                tauri::RunEvent::ExitRequested {
                    code: None, api, ..
                } => api.prevent_exit(),
                tauri::RunEvent::Exit => hotkeys::shutdown(app),
                _ => {}
            }
            #[cfg(not(windows))]
            let _ = (app, event);
        });
}
