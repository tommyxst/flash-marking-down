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
mod memory;
mod model;
mod platform;
#[cfg(windows)]
mod settings;
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
            m2_commands::init_organize_worker(app.handle());
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
            desktop_commands::read_workspace_image,
            credential_commands::set_credential,
            m2_commands::get_model_config,
            m2_commands::get_integrations_config,
            m2_commands::save_model_config,
            m2_commands::save_memory_config,
            m2_commands::test_model_connection,
            m2_commands::list_remote_models,
            m2_commands::test_memory_connection,
            m2_commands::list_notes,
            m2_commands::get_note_detail,
            m2_commands::trigger_note_organize,
            m2_commands::retry_memory_sync,
            m2_commands::drawer_counts,
            m2_commands::list_conflicts,
            m2_commands::create_conflict,
            m2_commands::resolve_conflict,
            m2_commands::wiki_targets,
            m2_commands::map_wiki_target,
            m2_commands::unmap_wiki_target,
            m2_commands::create_wiki_target,
            m2_commands::list_wiki_suggestions,
            m2_commands::create_wiki_suggestion,
            m2_commands::apply_wiki_suggestion,
            m2_commands::recompare_wiki_suggestion,
            m2_commands::reject_wiki_suggestion,
            m2_commands::undo_wiki_suggestion,
            m2_commands::list_tasks,
            m2_commands::toggle_task,
            m2_commands::list_conversations,
            m2_commands::get_conversation,
            m2_commands::delete_conversation,
            m2_commands::send_chat_message,
            m2_commands::read_memory_item,
            settings::get_app_settings,
            settings::save_app_settings,
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
