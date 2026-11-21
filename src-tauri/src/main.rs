#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod errors;
mod profiles;
mod rdp;
mod store;
mod trust;
mod vault;

use std::sync::Arc;

use tauri::Manager;

use rdp::clipboard::OsClipboard;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let staging_root = app
                .path()
                .app_cache_dir()
                .expect("app cache dir")
                .join(rdp::clipboard::STAGING_DIR_NAME);
            #[cfg(windows)]
            let os: Arc<dyn OsClipboard + Send + Sync> = Arc::new(rdp::clipboard::WindowsOs);
            #[cfg(not(windows))]
            let os: Arc<dyn OsClipboard + Send + Sync> = Arc::new(rdp::clipboard::UnsupportedOs);
            app.manage(rdp::Sessions::with_clipboard_pump(os, staging_root));
            Ok(())
        })
        .manage(vault::Vault::default())
        .invoke_handler(tauri::generate_handler![
            rdp::rdp_connect,
            rdp::trust_host,
            rdp::rdp_disconnect,
            rdp::rdp_status,
            rdp::rdp_frame,
            rdp::rdp_input,
            rdp::rdp_screenshot,
            profiles::list_profiles,
            profiles::save_profile,
            profiles::remove_profile,
            vault::vault_status,
            vault::vault_create,
            vault::vault_unlock,
            vault::vault_lock,
            vault::vault_change_password,
            vault::vault_clear,
            vault::vault_set_password,
            vault::vault_remove_password,
            vault::vault_get_password
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AuuRemote");
}
