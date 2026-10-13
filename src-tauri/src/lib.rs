use serde::Serialize;

#[derive(Serialize)]
struct CoreInfo {
    version: &'static str,
    engines: Vec<dbcore::Engine>,
}

/// Round-trip check that the UI can reach the Rust core.
#[tauri::command]
fn core_info() -> CoreInfo {
    CoreInfo {
        version: dbcore::VERSION,
        engines: dbcore::Engine::ALL.to_vec(),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![core_info])
        .run(tauri::generate_context!())
        .expect("error while running tablesoft");
}
