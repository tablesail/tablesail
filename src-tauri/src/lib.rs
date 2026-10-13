mod assistant;
mod commands;
mod jobs;
mod keychain;
mod runner;
mod schema;
mod state;
mod store;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::default())
        .setup(|app| {
            let paths = app.path();
            app.state::<AppState>()
                .init(paths.app_config_dir()?, paths.app_cache_dir()?);
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::connections_list,
            commands::connection_save,
            commands::connection_delete,
            commands::connection_test,
            commands::connection_open,
            commands::connection_close,
            commands::schema_databases,
            commands::schema_schemas,
            commands::schema_tables,
            commands::schema_snapshot,
            commands::table_structure,
            commands::browse_page,
            commands::browse_count,
            commands::cell_fetch,
            commands::sql_split,
            commands::query_run,
            commands::query_status,
            commands::query_fetch,
            commands::query_cancel,
            commands::query_close,
            commands::changes_preview,
            commands::changes_apply,
            commands::export_start,
            commands::csv_preview,
            commands::import_start,
            commands::job_status,
            commands::job_cancel,
            commands::query_log_since,
            commands::query_log_clear,
            assistant::assistant_ask,
            assistant::assistant_cancel,
            assistant::assistant_key_status,
            assistant::assistant_key_set,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tablesoft");
}
