//! Boîte à outils — backend Tauri.
#![warn(clippy::cognitive_complexity)]

pub mod cancel;
pub mod checksum;
pub mod commands;
pub mod download;
pub mod extract;
pub mod jobs;
pub mod output;
pub mod registry;
pub mod sidecar;
pub mod timeutil;
pub mod tools;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_tools,
            commands::check_tools,
            commands::run_tool,
            commands::cancel_job,
            commands::download_binary
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application");
}
