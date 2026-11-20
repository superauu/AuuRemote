#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod profiles;
mod rdp;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(rdp::Sessions::default())
        .invoke_handler(tauri::generate_handler![
            rdp::rdp_connect,
            rdp::rdp_disconnect,
            rdp::rdp_status,
            rdp::rdp_frame,
            rdp::rdp_input,
            rdp::rdp_screenshot,
            profiles::list_profiles,
            profiles::save_profile,
            profiles::remove_profile
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AuuRemote");
}
