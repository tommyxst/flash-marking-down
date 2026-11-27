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
mod wikra_commands;
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
            wikra_commands::init_buffer_worker(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            capture_commands::open_workspace,
            capture_commands::init_standard_vault,
            capture_commands::update_workspace_dirs,
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
            capture_commands::commit_capture_v2,
            capture_commands::confirm_capture,
            desktop_commands::pick_workspace_folder,
            desktop_commands::desktop_window_action,
            desktop_commands::is_main_maximized,
            desktop_commands::open_workspace_file,
            desktop_commands::open_external_url,
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
            m2_commands::update_note,
            m2_commands::trigger_note_organize,
            m2_commands::retry_memory_sync,
            m2_commands::drawer_counts,
            m2_commands::list_conflicts,
            m2_commands::create_conflict,
            m2_commands::resolve_conflict,
            m2_commands::list_wiki_documents,
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
            wikra_commands::list_library_tree,
            wikra_commands::read_document_detail,
            wikra_commands::save_markdown_document,
            wikra_commands::create_markdown_document,
            wikra_commands::append_human_understanding,
            wikra_commands::list_inbox_items,
            wikra_commands::get_inbox_item,
            wikra_commands::ingest_inbox_input,
            wikra_commands::complete_inbox_stub,
            wikra_commands::toggle_inbox_paragraph,
            wikra_commands::compile_inbox_item,
            wikra_commands::list_organize_overview,
            wikra_commands::cancel_buffer_item,
            wikra_commands::flush_buffer_now,
            wikra_commands::revert_changeset,
            wikra_commands::dismiss_suggestion,
            wikra_commands::teachback_turn,
            wikra_commands::save_answer_to_wiki,
            wikra_commands::get_knowledge_graph,
            wikra_commands::dismiss_graph_gap,
            wikra_commands::global_search,
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
                tauri::RunEvent::Exit => {
                    wikra_commands::stop_buffer_worker(app);
                    hotkeys::shutdown(app);
                }
                _ => {}
            }
            #[cfg(not(windows))]
            let _ = (app, event);
        });
}
