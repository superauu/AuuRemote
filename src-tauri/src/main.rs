#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod errors;
mod profiles;
mod rdp;
mod store;
mod vault;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(rdp::Sessions::default())
        .manage(vault::Vault::default())
        .invoke_handler(tauri::generate_handler![
            rdp::rdp_connect,
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
