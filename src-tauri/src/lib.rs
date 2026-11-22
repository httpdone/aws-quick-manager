mod quick;

use quick::{accounts, credstore, profiles};

#[tauri::command]
fn list_accounts() -> Result<Vec<accounts::AccountView>, String> {
    accounts::list(&profiles::base_dir(), credstore::system().as_ref()).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![list_accounts])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
