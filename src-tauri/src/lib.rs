//! Point d'entrée de la bibliothèque IH Desktop (Tauri).

mod commands;
mod logging;
mod state;

use ih_core::AppPaths;
use state::AppState;
use tauri::Manager;

/// Démarre IH.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let paths = AppPaths::resolve()?;
            let handle = app.handle().clone();
            let state = AppState::init(handle, paths)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // noyau
            commands::core::app_health,
            commands::core::platform_info,
            commands::core::config_get,
            commands::core::config_update,
            commands::core::locale_set,
            commands::core::paths_info,
            commands::core::secret_store_status,
            commands::core::classify_command_preview,
            commands::core::approvals_pending,
            commands::core::approval_resolve,
            // projets & fichiers
            commands::core::project_add,
            commands::core::project_list,
            commands::core::project_remove,
            commands::core::file_tree,
            commands::core::file_read,
            commands::core::file_write,
            commands::core::file_search,
            // workspace, contexte et diff
            commands::files::context_settings,
            commands::files::workspace_meta,
            commands::files::workspace_save,
            commands::files::files_match,
            commands::files::context_prepare,
            commands::files::diff_prepare,
            commands::files::diff_pending,
            commands::files::diff_accept,
            commands::files::diff_reject,
            commands::files::diff_view,
            commands::files::layout_get,
            commands::files::layout_save,
            // diagnostic
            commands::core::log_tail,
            commands::core::diagnostics_report,
            commands::core::open_external,
            commands::core::open_path,
            // git & terminal
            commands::workspace::git_status,
            commands::workspace::git_diff,
            commands::workspace::git_stage,
            commands::workspace::git_unstage,
            commands::workspace::git_commit,
            commands::workspace::git_branches,
            commands::workspace::git_log,
            commands::workspace::git_worktrees,
            commands::workspace::git_add_worktree,
            commands::workspace::git_remove_worktree,
            commands::workspace::terminal_open,
            commands::workspace::terminal_write,
            commands::workspace::terminal_resize,
            commands::workspace::terminal_close,
            // agents & fournisseurs
            commands::ai::agents_list,
            commands::ai::agent_start,
            commands::ai::agent_cancel,
            commands::ai::agent_state,
            commands::ai::agent_sessions,
            commands::ai::providers_list,
            commands::ai::provider_set_secret,
            commands::ai::provider_clear_secret,
            commands::ai::provider_health,
            commands::ai::models_list,
            commands::ai::provider_set_default_model,
            commands::ai::provider_set_enabled,
            commands::ai::provider_create,
            commands::ai::provider_update,
            commands::ai::provider_delete,
            commands::ai::provider_test,
            commands::ai::provider_add_model,
            commands::ai::provider_remove_model,
            // conversation
            commands::chat::chat_send,
            commands::chat::chat_cancel,
            commands::chat::chat_active,
            commands::chat::thread_rename,
            commands::chat::threads_search,
            // mode agent
            commands::agent::agent_adapters,
            commands::agent::agent_session_request_approval,
            commands::agent::agent_session_start,
            commands::agent::agent_session_cancel,
            commands::agent::agent_session_list,
            commands::agent::agent_session_detail,
            commands::agent::agent_session_events,
            commands::agent::agent_session_refuse,
            // mode équipe
            commands::team::team_request_approval,
            commands::team::team_run,
            // MCP distant (Streamable HTTP / SSE)
            commands::mcp::mcp_remote_servers_list,
            commands::mcp::mcp_server_upsert,
            commands::mcp::mcp_server_delete,
            commands::mcp::mcp_server_set_enabled,
            commands::mcp::mcp_server_set_secret,
            commands::mcp::mcp_server_clear_secret,
            commands::mcp::mcp_server_connect,
            commands::mcp::mcp_server_disconnect,
            commands::mcp::mcp_server_test,
            commands::mcp::mcp_server_refresh_tools,
            commands::mcp::mcp_tools_list,
            commands::mcp::mcp_tool_set_enabled,
            commands::mcp::mcp_call,
            commands::mcp::mcp_calls_list,
            commands::mcp::mcp_export_config,
            // skills & MCP stdio (legacy)
            commands::extensions::skills_list,
            commands::extensions::skill_install,
            commands::extensions::skill_verify,
            commands::extensions::skill_remove,
            commands::extensions::skill_set_enabled,
            commands::extensions::mcp_servers_list,
            commands::extensions::mcp_server_add,
            commands::extensions::mcp_server_remove,
            commands::extensions::mcp_connect,
            commands::extensions::mcp_call_tool,
            commands::extensions::mcp_disconnect,
            // tâches, mémoire, historique, automatisations, mises à jour
            commands::ops::tasks_list,
            commands::ops::task_create,
            commands::ops::task_set_state,
            commands::ops::plan_create,
            commands::ops::memory_list,
            commands::ops::memory_add,
            commands::ops::memory_search,
            commands::ops::memory_delete,
            commands::ops::memory_preview,
            commands::ops::threads_list,
            commands::ops::thread_create,
            commands::ops::messages_list,
            commands::ops::thread_delete,
            commands::ops::history_export,
            commands::ops::workflows_list,
            commands::ops::workflow_upsert,
            commands::ops::workflow_run_now,
            commands::ops::workflow_runs,
            commands::ops::app_version,
            commands::ops::update_check,
            commands::ops::update_status,
            commands::ops::update_download,
            commands::ops::update_plan,
            commands::ops::update_apply,
            commands::ops::update_request_approval,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement d'IH");
}
