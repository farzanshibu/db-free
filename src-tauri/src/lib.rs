// SOT: app-entry, tauri-builder, command-registration, module-tree
// Tests may panic on setup failure; production code may not (Cargo.toml [lints]).
#![cfg_attr(test, allow(clippy::panic, clippy::unwrap_used, clippy::expect_used))]

pub mod adapters;
pub mod commands;
pub mod integrations;
pub mod error;
pub mod guard;
pub mod model;
pub mod services;
pub mod state;
pub mod store;
pub mod open_files;
pub mod diagnostics;
pub mod menu;

use adapters::keyring::OsKeyring;
use state::AppState;
use store::Store;
use tauri::Manager;

// WHAT:  Builds the Tauri app: opens the local store, manages AppState, registers
//        every command.
// WHY:   Startup work is minimal on purpose (cold start budget < 150 ms): the
//        master key and integration sessions are created lazily on first use.
// WHERE: src-tauri/src/commands/mod.rs (CommandName must match this list)
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Logging is the log plugin's job (src-tauri/src/diagnostics.rs): it owns the global logger.
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        // A second launch (Windows / Linux "Open with", a double-click on a database
        // file) lands in the running app: open the files it was given and focus the window.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            open_files::on_second_launch(app, &argv);
        }))
        .plugin(diagnostics::log_plugin())
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED
                        | tauri_plugin_window_state::StateFlags::FULLSCREEN,
                )
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let store = Store::open(&data_dir.join("db-free.sqlite"))?;
            // The master key lives in the OS keychain; on a machine without one it falls back to a 0600 file.
            let keys = OsKeyring::default().with_file_fallback(data_dir.join("master.key"));
            app.manage(AppState::new(store, Box::new(keys)));
            // Holds an update downloaded in the background until the user restarts.
            app.manage(commands::updates::StagedUpdate::default());
            // Cancel switches for backup / restore runs in flight.
            app.manage(commands::backup::BackupRuns::default());
            // Workflow runs in flight, then the clock that fires Schedule nodes with every window closed.
            app.manage(services::workflows::WorkflowRuns::default());
            commands::workflows::start_scheduler(app.handle().clone());
            // Settings → Editor → Keep query log for: trim the log once, at startup.
            if let Some(state) = app.try_state::<AppState>() {
                services::history::prune_on_start(&state);
            }
            // Files opened for one session never outlive the one that opened them.
            open_files::sweep_stale_transients(app.handle());
            // Windows / Linux pass a double-clicked file as an argument of the first launch.
            open_files::open_from_args(app.handle(), std::env::args_os().skip(1));
            // Chords last sent for the native menu; the UI refreshes them on startup.
            app.manage(commands::windows::MenuShortcuts::default());
            diagnostics::install_panic_hook(app.handle().clone());
            app.set_menu(menu::build(app.handle(), &std::collections::BTreeMap::new())?)?;
            app.on_menu_event(|handle, event| menu::dispatch(handle, event.id().as_ref()));
            // The schema cache's background refresh tells the UI when it found the
            // catalog different from the copy the sidebar was drawn from.
            if let Some(state) = app.try_state::<AppState>() {
                let handle = app.handle().clone();
                state.schema_cache().set_notifier(move |event| {
                    if let Err(err) = tauri::Emitter::emit(&handle, "schema-cache:updated", event) {
                        log::debug!("schema-cache:updated not delivered: {err}");
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connections::list_connections,
            commands::connections::save_connection,
            commands::connections::delete_connection,
            commands::connections::test_connection,
            commands::connections::connect,
            commands::connections::disconnect,
            commands::connections::active_sessions,
            commands::connections::describe_session,
            commands::schema::load_catalog,
            commands::schema::load_columns,
            commands::data::fetch_table_page,
            commands::query::execute_query,
            commands::query::list_history,
            commands::query::list_buffers,
            commands::query::save_buffer,
            commands::query::delete_buffer,
            commands::query::split_script,
            commands::query::save_sql_file,
            commands::query::clear_history,
            commands::schema::load_foreign_keys,
            commands::schema::load_ddl,
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::library::list_saved_queries,
            commands::library::save_saved_query,
            commands::library::delete_saved_query,
            commands::library::list_documents,
            commands::library::save_document,
            commands::library::delete_document,
            commands::changes::preview_changes,
            commands::changes::commit_changes,
            commands::transfer::export_tables,
            commands::transfer::import_file,
            commands::ai::ai_generate,
            commands::ai::explain_query,
            commands::workflows::run_workflow,
            commands::objects::list_objects,
            commands::objects::load_object,
            commands::objects::server_stats,
            commands::objects::vector_search,
            commands::objects::search_documents,
            commands::objects::query_range,
            commands::objects::load_history,
            commands::objects::download_object,
            commands::updates::check_update,
            commands::updates::download_update,
            commands::updates::install_update,
            commands::schema::create_template,
            commands::agent::agent_chat,
            commands::agent::agent_decide,
            commands::agent::agent_cancel,
            commands::agent::agent_reset,
            commands::agent::agent_skills,
            commands::backup::detect_native_tools,
            commands::backup::backup_database,
            commands::backup::restore_database,
            commands::backup::cancel_backup,
            commands::query::cancel_query,
            commands::query::begin_transaction,
            commands::query::commit_transaction,
            commands::query::rollback_transaction,
            commands::connections::ping_session,
            commands::compare::schema_diff,
            commands::compare::compare_table_data,
            commands::workflows::cancel_workflow,
            commands::workflows::list_workflow_schedules,
            commands::workflows::test_workflow_regex,
            commands::workflows::workflow_last_node_config,
            commands::diagrams::list_diagram_customizations,
            commands::diagrams::save_diagram_customizations,
            commands::diagrams::autosave_document,
            commands::diagrams::checkpoint_document,
            commands::diagrams::list_document_versions,
            commands::diagrams::restore_document_version,
            commands::diagrams::delete_document_version,
            commands::diagrams::diagram_ddl,
            commands::diagrams::save_diagram_export,
            commands::library::list_library,
            commands::library::create_library_folder,
            commands::library::rename_library_folder,
            commands::library::move_library_folder,
            commands::library::delete_library_folder,
            commands::library::move_library_item,
            commands::library::rename_document,
            commands::library::duplicate_document,
            commands::library::rename_saved_query,
            commands::library::duplicate_saved_query,
            commands::library::list_saved_query_versions,
            commands::library::restore_saved_query_version,
            commands::library::export_saved_queries,
            commands::library::list_pins,
            commands::library::pin_item,
            commands::library::unpin_item,
            commands::library::reorder_pins,
            commands::library::list_recents,
            commands::library::record_recent,
            commands::library::clear_recents,
            commands::changes::rows_as_sql,
            commands::data::fetch_rows,
            commands::data::list_fk_remaps,
            commands::data::save_fk_remap,
            commands::data::delete_fk_remap,
            commands::tags::list_table_tags,
            commands::tags::save_table_tag,
            commands::tags::delete_table_tag,
            commands::tags::reorder_table_tags,
            commands::tags::tag_table,
            commands::tags::set_tag_collapsed,
            commands::tags::move_tagged_table,
            commands::schema::ddl_profile,
            commands::schema::describe_table,
            commands::schema::preview_ddl,
            commands::query::execute_batch,
            commands::query::list_history_page,
            commands::query::list_editor_history,
            commands::query::read_sql_file,
            commands::keys::scan_keys,
            commands::keys::key_info,
            commands::transfer::preview_export,
            commands::transfer::preview_import,
            commands::transfer::plan_json_import,
            commands::transfer::run_json_import,
            commands::transfer::inspect_sql_file,
            commands::transfer::import_sql_file,
            commands::connections::duplicate_connection,
            commands::connections::validate_connection,
            commands::connections::credential_status,
            commands::connections::reset_credentials,
            commands::connection_org::list_connection_org,
            commands::connection_org::save_connection_tag,
            commands::connection_org::delete_connection_tag,
            commands::connection_org::save_connection_folder,
            commands::connection_org::delete_connection_folder,
            commands::connection_org::set_connection_org,
            commands::connection_files::export_connections,
            commands::connection_files::import_connections,
            commands::connection_files::open_database_file,
            commands::connection_files::save_transient_connection,
            commands::connection_files::list_ssh_config_hosts,
            commands::connection_files::create_demo_connection,
            commands::windows::open_window,
            commands::windows::close_window,
            commands::windows::set_ui_zoom,
            commands::windows::set_window_theme,
            commands::windows::open_external,
            commands::windows::open_log_folder,
            commands::windows::log_ui_error,
            commands::windows::set_menu_shortcuts,
            commands::windows::export_theme_file,
            commands::windows::import_theme_file,
            commands::updates::app_info,
            commands::ai::ai_settings_save,
            commands::ai::ai_models,
            commands::ai::ai_test,
            commands::ai::ai_chat_list,
            commands::ai::ai_chat_load,
            commands::ai::ai_chat_rename,
            commands::ai::ai_chat_delete,
            commands::ai::ai_diagram_from_schema,
            commands::sessions::close_session,
        ])
        .build(tauri::generate_context!())?
        .run(|app, event| open_files::on_run_event(app, &event));
    Ok(())
}

// WHAT:  Writes the TS bindings for every `#[ts(export)]` type.
// HOW:   `pnpm bindings` sets TS_RS_EXPORT_DIR=../src/lib/bindings and runs this test.
#[cfg(test)]
mod export_bindings {
    use ts_rs::TS;

    // Roots only: `export_all` follows every dependency type.
    // i64/u64 map to `number` because serde_json emits plain JSON numbers.
    #[test]
    fn export_bindings() {
        let cfg = ts_rs::Config::from_env().with_large_int("number");
        let results = [
            crate::error::AppError::export_all(&cfg),
            crate::commands::CommandName::export_all(&cfg),
            crate::commands::connections::SaveConnectionRequest::export_all(&cfg),
            crate::commands::connections::ConnectionIdRequest::export_all(&cfg),
            crate::commands::connections::ConnectRequest::export_all(&cfg),
            crate::commands::connections::SessionRequest::export_all(&cfg),
            crate::commands::schema::CatalogRequest::export_all(&cfg),
            crate::commands::schema::ColumnsRequest::export_all(&cfg),
            crate::commands::schema::CreateTemplateRequest::export_all(&cfg),
            crate::commands::data::TablePageRequest::export_all(&cfg),
            crate::commands::query::ExecuteQueryRequest::export_all(&cfg),
            crate::commands::query::HistoryRequest::export_all(&cfg),
            crate::commands::query::SaveBufferRequest::export_all(&cfg),
            crate::commands::query::BufferIdRequest::export_all(&cfg),
            crate::commands::query::SplitScriptRequest::export_all(&cfg),
            crate::commands::query::SaveSqlFileRequest::export_all(&cfg),
            crate::model::StatementSpan::export_all(&cfg),
            crate::integrations::SessionInfo::export_all(&cfg),
            crate::model::EngineKind::export_all(&cfg),
            crate::model::FormKind::export_all(&cfg),
            crate::model::EngineFacts::export_all(&cfg),
            crate::model::ConnectionSummary::export_all(&cfg),
            crate::model::SchemaCatalog::export_all(&cfg),
            crate::model::TablePage::export_all(&cfg),
            crate::model::QueryOutcome::export_all(&cfg),
            crate::model::HistoryEntry::export_all(&cfg),
            crate::model::EditorBuffer::export_all(&cfg),
            crate::commands::query::ClearHistoryRequest::export_all(&cfg),
            crate::commands::settings::SaveSettingsRequest::export_all(&cfg),
            crate::model::UpdateStatus::export_all(&cfg),
            crate::model::UpdateProgress::export_all(&cfg),
            crate::commands::library::SaveQueryRequest::export_all(&cfg),
            crate::commands::library::IdRequest::export_all(&cfg),
            crate::commands::library::ListDocumentsRequest::export_all(&cfg),
            crate::commands::library::SaveDocumentRequest::export_all(&cfg),
            crate::commands::changes::ChangesRequest::export_all(&cfg),
            crate::commands::transfer::ExportRequest::export_all(&cfg),
            crate::commands::transfer::ImportRequest::export_all(&cfg),
            crate::commands::ai::AiGenerateRequest::export_all(&cfg),
            crate::commands::ai::ExplainRequest::export_all(&cfg),
            crate::commands::workflows::RunWorkflowRequest::export_all(&cfg),
            crate::model::ForeignKey::export_all(&cfg),
            crate::model::ChangePreview::export_all(&cfg),
            crate::model::ExportReport::export_all(&cfg),
            crate::model::ImportReport::export_all(&cfg),
            crate::model::AiReply::export_all(&cfg),
            crate::model::PlanReport::export_all(&cfg),
            crate::model::WorkflowRunReport::export_all(&cfg),
            crate::model::AppSettings::export_all(&cfg),
            crate::model::SavedQuery::export_all(&cfg),
            crate::model::Document::export_all(&cfg),
            crate::commands::objects::ObjectsRequest::export_all(&cfg),
            crate::commands::objects::ObjectRequest::export_all(&cfg),
            crate::commands::objects::VectorSearchCommand::export_all(&cfg),
            crate::commands::objects::SearchCommand::export_all(&cfg),
            crate::commands::objects::RangeQueryCommand::export_all(&cfg),
            crate::commands::objects::DownloadObjectRequest::export_all(&cfg),
            crate::commands::objects::DownloadObjectReport::export_all(&cfg),
            crate::integrations::FamilyProfile::export_all(&cfg),
            crate::model::ObjectDetail::export_all(&cfg),
            crate::model::ServerStats::export_all(&cfg),
            crate::model::SearchResult::export_all(&cfg),
            crate::model::RangeResult::export_all(&cfg),
            crate::commands::agent::AgentChatRequest::export_all(&cfg),
            crate::commands::agent::AgentDecisionRequest::export_all(&cfg),
            crate::commands::agent::AgentRunRequest::export_all(&cfg),
            crate::commands::agent::AgentChatIdRequest::export_all(&cfg),
            // Only reachable through the event stream, so it needs naming here.
            crate::model::AgentEvent::export_all(&cfg),
            crate::model::AgentTurn::export_all(&cfg),
            crate::model::AgentSkill::export_all(&cfg),
            crate::model::AgentAutonomy::export_all(&cfg),
            crate::commands::backup::DetectToolsRequest::export_all(&cfg),
            crate::commands::backup::BackupRequest::export_all(&cfg),
            crate::commands::backup::RestoreRequest::export_all(&cfg),
            crate::commands::backup::BackupRunRequest::export_all(&cfg),
            crate::model::BackupSupport::export_all(&cfg),
            crate::model::BackupReport::export_all(&cfg),
            // Only reachable through the event stream, so it needs naming here.
            crate::model::BackupEvent::export_all(&cfg),
            crate::commands::query::CancelQueryRequest::export_all(&cfg),
            crate::commands::compare::SchemaDiffRequest::export_all(&cfg),
            crate::model::SchemaDiff::export_all(&cfg),
            crate::commands::compare::CompareTableDataRequest::export_all(&cfg),
            crate::model::DataCompare::export_all(&cfg),
            crate::commands::workflows::CancelWorkflowRequest::export_all(&cfg),
            crate::commands::workflows::TestRegexRequest::export_all(&cfg),
            crate::commands::workflows::LastNodeConfigRequest::export_all(&cfg),
            // Only reachable through the event stream, so it needs naming here.
            crate::model::WorkflowEvent::export_all(&cfg),
            crate::model::WorkflowScheduleStatus::export_all(&cfg),
            crate::model::RegexTestResult::export_all(&cfg),
            crate::model::NodeConfig::export_all(&cfg),
            crate::commands::diagrams::DiagramScopeRequest::export_all(&cfg),
            crate::commands::diagrams::SaveDiagramCustomizationsRequest::export_all(&cfg),
            crate::commands::diagrams::AutosaveDocumentRequest::export_all(&cfg),
            crate::commands::diagrams::CheckpointDocumentRequest::export_all(&cfg),
            crate::commands::diagrams::DocumentHistoryRequest::export_all(&cfg),
            crate::commands::diagrams::DocumentVersionRequest::export_all(&cfg),
            crate::commands::diagrams::DiagramDdlRequest::export_all(&cfg),
            crate::commands::diagrams::SaveDiagramExportRequest::export_all(&cfg),
            crate::model::DocumentVersionSummary::export_all(&cfg),
            crate::model::DiagramDdl::export_all(&cfg),
            crate::commands::library::RenameRequest::export_all(&cfg),
            crate::commands::library::CreateFolderRequest::export_all(&cfg),
            crate::commands::library::MoveFolderRequest::export_all(&cfg),
            crate::commands::library::MoveItemRequest::export_all(&cfg),
            crate::commands::library::DuplicateDocumentRequest::export_all(&cfg),
            crate::commands::library::DuplicateQueryRequest::export_all(&cfg),
            crate::commands::library::ExportQueriesRequest::export_all(&cfg),
            crate::commands::library::HomeEntityRequest::export_all(&cfg),
            crate::commands::library::UnpinRequest::export_all(&cfg),
            crate::commands::library::ReorderPinsRequest::export_all(&cfg),
            crate::model::LibraryLayout::export_all(&cfg),
            crate::model::SavedQueryVersion::export_all(&cfg),
            crate::model::HomePin::export_all(&cfg),
            crate::model::HomeRecent::export_all(&cfg),
            crate::commands::changes::RowsSqlRequest::export_all(&cfg),
            crate::commands::data::FetchRowsRequest::export_all(&cfg),
            crate::commands::data::SaveFkRemapRequest::export_all(&cfg),
            crate::commands::data::DeleteFkRemapRequest::export_all(&cfg),
            crate::model::CommitReport::export_all(&cfg),
            crate::model::FkRemap::export_all(&cfg),
            crate::commands::tags::ListTagsRequest::export_all(&cfg),
            crate::commands::tags::SaveTagRequest::export_all(&cfg),
            crate::commands::tags::ReorderTagsRequest::export_all(&cfg),
            crate::commands::tags::TagTableRequest::export_all(&cfg),
            crate::commands::tags::TagCollapseRequest::export_all(&cfg),
            crate::commands::tags::MoveTaggedTableRequest::export_all(&cfg),
            crate::commands::schema::PreviewDdlRequest::export_all(&cfg),
            crate::model::TableTag::export_all(&cfg),
            crate::model::TagState::export_all(&cfg),
            crate::model::DdlProfile::export_all(&cfg),
            crate::model::TableSpec::export_all(&cfg),
            crate::model::DdlPreview::export_all(&cfg),
            crate::commands::query::ExecuteBatchRequest::export_all(&cfg),
            crate::commands::query::HistoryPageRequest::export_all(&cfg),
            crate::commands::query::EditorHistoryRequest::export_all(&cfg),
            crate::model::HistoryPage::export_all(&cfg),
            crate::commands::query::ReadSqlFileRequest::export_all(&cfg),
            crate::model::OpenedSqlFile::export_all(&cfg),
            // Only reachable through the batch's progress channel, so it needs naming here.
            crate::model::BatchEvent::export_all(&cfg),
            crate::model::BatchSummary::export_all(&cfg),
            crate::commands::keys::ScanKeysRequest::export_all(&cfg),
            crate::commands::keys::KeyInfoRequest::export_all(&cfg),
            crate::model::RedisKeyPage::export_all(&cfg),
            crate::model::RedisKeyInfo::export_all(&cfg),
            crate::commands::transfer::PreviewExportRequest::export_all(&cfg),
            crate::commands::transfer::PreviewImportRequest::export_all(&cfg),
            crate::commands::transfer::JsonPlanRequest::export_all(&cfg),
            crate::commands::transfer::JsonImportRequest::export_all(&cfg),
            crate::commands::transfer::SqlFileRequest::export_all(&cfg),
            crate::commands::transfer::SqlImportRequest::export_all(&cfg),
            crate::model::ExportPreview::export_all(&cfg),
            crate::model::ImportPreview::export_all(&cfg),
            crate::model::JsonImportPlan::export_all(&cfg),
            crate::model::JsonImportReport::export_all(&cfg),
            crate::model::SqlFileInfo::export_all(&cfg),
            crate::model::SqlImportReport::export_all(&cfg),
            // Only reachable through the progress channels, so they need naming here.
            crate::model::ExportProgress::export_all(&cfg),
            crate::model::ImportProgress::export_all(&cfg),
            crate::model::SqlImportProgress::export_all(&cfg),
            crate::commands::connection_org::SaveConnectionTagRequest::export_all(&cfg),
            crate::commands::connection_org::SaveConnectionFolderRequest::export_all(&cfg),
            crate::commands::connection_org::OrgIdRequest::export_all(&cfg),
            crate::commands::connection_org::SetConnectionOrgRequest::export_all(&cfg),
            crate::commands::connection_files::ExportConnectionsRequest::export_all(&cfg),
            crate::commands::connection_files::ImportConnectionsRequest::export_all(&cfg),
            crate::commands::connection_files::OpenDatabaseFileRequest::export_all(&cfg),
            crate::commands::connection_files::SaveTransientRequest::export_all(&cfg),
            crate::model::ConnectionFieldIssue::export_all(&cfg),
            crate::model::ConnectionOrg::export_all(&cfg),
            crate::model::ConnectionImportReport::export_all(&cfg),
            crate::model::ConnectionExportReport::export_all(&cfg),
            crate::model::ConnectionOpenedEvent::export_all(&cfg),
            crate::model::SshConfigHost::export_all(&cfg),
            crate::model::CredentialStatus::export_all(&cfg),
            crate::commands::windows::OpenWindowRequest::export_all(&cfg),
            crate::commands::windows::SetZoomRequest::export_all(&cfg),
            crate::commands::windows::SetWindowThemeRequest::export_all(&cfg),
            crate::commands::windows::OpenExternalRequest::export_all(&cfg),
            crate::commands::windows::LogUiErrorRequest::export_all(&cfg),
            crate::commands::windows::MenuShortcutsRequest::export_all(&cfg),
            crate::commands::windows::ExportThemeRequest::export_all(&cfg),
            crate::commands::windows::ImportThemeRequest::export_all(&cfg),
            crate::model::AppInfo::export_all(&cfg),
            crate::model::CustomTheme::export_all(&cfg),
            crate::model::ThemeBase::export_all(&cfg),
            // Only reachable through events, so they need naming here.
            crate::model::MenuActionEvent::export_all(&cfg),
            crate::model::AppErrorEvent::export_all(&cfg),
            crate::commands::ai::SaveAiSettingsRequest::export_all(&cfg),
            crate::commands::ai::AiModelsRequest::export_all(&cfg),
            crate::commands::ai::AiTestRequest::export_all(&cfg),
            crate::commands::ai::AiChatsRequest::export_all(&cfg),
            crate::commands::ai::AiChatRenameRequest::export_all(&cfg),
            crate::commands::ai::DiagramFromSchemaRequest::export_all(&cfg),
            crate::model::AiModelList::export_all(&cfg),
            crate::model::AiTestResult::export_all(&cfg),
            crate::model::AiChatSummary::export_all(&cfg),
            crate::model::AiChatDetail::export_all(&cfg),
            crate::model::AiDataAccess::export_all(&cfg),
            crate::model::ChatMention::export_all(&cfg),
            crate::model::ModelChoice::export_all(&cfg),
            crate::commands::sessions::CloseSessionRequest::export_all(&cfg),
            // Only reachable through an event, so it needs naming here.
            crate::model::SchemaCacheUpdated::export_all(&cfg),
        ];
        for result in results {
            result.unwrap_or_else(|e| panic!("{e}"));
        }
    }

    // WHAT:  Writes the per-engine facts Rust owns (category, form kind, default
    //        port) as a typed TS map next to the generated bindings.
    // WHY:   src/lib/engines.ts restates these for the picker and the connection
    //        form. Generating them makes any disagreement a compile error there
    //        instead of a wrong port or a missing category in the UI.
    #[test]
    fn export_engine_facts() {
        use crate::model::Engine;
        let Ok(dir) = std::env::var("TS_RS_EXPORT_DIR") else {
            return; // only runs under `pnpm bindings`
        };
        let mut out = String::from(
            "// This file is generated by the `export_engine_facts` test. Do not edit this file manually.\n\
             import type { EngineFacts } from \"./EngineFacts\";\n\
             import type { Engine } from \"./Engine\";\n\n\
             export const ENGINE_FACTS = {\n",
        );
        for engine in Engine::ALL {
            let f = engine.facts();
            let json = serde_json::to_string(&f).unwrap_or_default();
            out.push_str(&format!("  {}: {json},\n", engine.as_str()));
        }
        out.push_str("} as const satisfies Record<Engine, EngineFacts>;\n");
        let path = std::path::Path::new(&dir).join("EngineFacts.gen.ts");
        std::fs::write(&path, out).unwrap_or_else(|e| panic!("{e}"));
    }

    // WHAT:  Writes every family's static profile (capabilities, object kinds,
    //        tools) as a typed TS map, so the capability matrix and the sidebar
    //        know what an engine offers before (or without) connecting.
    // WHERE: src/lib/objects.ts (profileOf), src-tauri/src/integrations/mod.rs (profile)
    #[test]
    fn export_family_profiles() {
        use crate::model::{Engine, Family};
        let Ok(dir) = std::env::var("TS_RS_EXPORT_DIR") else {
            return; // only runs under `pnpm bindings`
        };
        let mut families: Vec<Family> = Vec::new();
        for engine in Engine::ALL {
            if !families.contains(&engine.family()) {
                families.push(engine.family());
            }
        }
        let mut out = String::from(
            "// This file is generated by the `export_family_profiles` test. Do not edit this file manually.\n\
             import type { FamilyProfile } from \"./FamilyProfile\";\n\
             import type { Family } from \"./Family\";\n\
             import type { ObjectKind } from \"./ObjectKind\";\n\n\
             export const FAMILY_PROFILES = {\n",
        );
        for family in families {
            let key = serde_json::to_string(&family).unwrap_or_default().trim_matches('"').to_string();
            let json = serde_json::to_string(&crate::integrations::profile(family)).unwrap_or_default();
            out.push_str(&format!("  {key}: {json},\n"));
        }
        out.push_str("} satisfies Record<Family, FamilyProfile>;\n\n");
        out.push_str("// Kinds listed per namespace (schema / keyspace / database) rather than once per server.\n");
        out.push_str("export const SCOPED_OBJECT_KINDS: readonly ObjectKind[] = [");
        let scoped: Vec<String> = crate::model::ObjectKind::ALL
            .iter()
            .filter(|k| k.scoped())
            .map(|k| serde_json::to_string(k).unwrap_or_default())
            .collect();
        out.push_str(&scoped.join(", "));
        out.push_str("];\n");
        let path = std::path::Path::new(&dir).join("FamilyProfiles.gen.ts");
        std::fs::write(&path, out).unwrap_or_else(|e| panic!("{e}"));
    }
}
