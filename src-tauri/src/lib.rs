pub mod quick;
mod updates;

use quick::{accounts, api, credstore, desktop, device_ids, manage, oauth, profiles, switcher, vault};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

#[tauri::command]
fn list_accounts() -> Result<Vec<accounts::AccountView>, String> {
    accounts::list(&profiles::base_dir(), &vault::dir(), credstore::system().as_ref()).map_err(|e| e.to_string())
}

/// Copy every readable desktop session into the account vault.
#[tauri::command(async)]
fn sync_from_desktop() -> Result<vault::ImportReport, String> {
    vault::import_from_desktop(&profiles::base_dir(), credstore::system().as_ref(), &vault::dir()).map_err(|e| e.to_string())
}

/// Remove an account from the vault: index entry and stored credentials.
#[tauri::command(async)]
fn remove_from_vault(id: String) -> Result<(), String> {
    vault::remove(&vault::dir(), credstore::system().as_ref(), &id).map_err(|e| e.to_string())
}

/// Force-kill Quick after a polite quit timed out; the caller retries the
/// switch once this returns.
#[tauri::command(async)]
fn force_quit_desktop() -> Result<bool, String> {
    match desktop::find().map_err(|e| e.to_string())? {
        None => Ok(false),
        Some(p) => {
            desktop::force_quit(&p).map_err(|e| e.to_string())?;
            Ok(true)
        }
    }
}

#[tauri::command(async)]
fn desktop_status() -> Result<desktop::Status, String> {
    desktop::status().map_err(|e| e.to_string())
}

/// Switch the desktop to `id`: refresh its tokens, write them where the
/// desktop reads them, and start it again when `launch`.
#[tauri::command(async)]
async fn switch_account(id: String, launch: bool) -> Result<switcher::SwitchReport, String> {
    let base = profiles::base_dir();
    let backups = switcher::backups_dir();
    let vault_dir = vault::dir();
    let blob = switcher::refresh_target(&base, &id, &vault_dir).await.map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let store = credstore::system();
        switcher::commit(&base, &backups, &id, launch, store.as_ref(), &vault_dir, &blob)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

/// Undo the last switch (the desktop must not be running).
#[tauri::command(async)]
fn restore_last_backup() -> Result<String, String> {
    switcher::restore_latest(&profiles::base_dir(), &switcher::backups_dir())
        .map(|p| p.display().to_string())
        .map_err(|e| e.to_string())
}

/// Add an account through the browser sign-in, then write it to the desktop.
#[tauri::command(async)]
async fn login_account() -> Result<oauth::LoginOutcome, String> {
    let account = oauth::login().await.map_err(|e| e.to_string())?;
    let base = profiles::base_dir();
    let vault_dir = vault::dir();
    tauri::async_runtime::spawn_blocking(move || {
        let store = credstore::system();
        oauth::commit(&base, store.as_ref(), &vault_dir, account)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
struct BackupAccount {
    id: String,
    display_name: String,
    active: bool,
}

#[derive(serde::Serialize)]
struct BackupInfo {
    name: String,
    modified: i64,
    accounts: usize,
    active: String,
    active_changed: bool,
    current_active: String,
    /// accounts the restore would bring back
    added: Vec<String>,
    /// accounts the restore would drop
    removed: Vec<String>,
    entries: Vec<BackupAccount>,
    parse_error: Option<String>,
}

fn manifest_accounts(manifest: &profiles::Manifest) -> Vec<BackupAccount> {
    let active = manifest.last_active.as_deref();
    manifest
        .entries
        .iter()
        .map(|e| BackupAccount {
            id: e.id.clone(),
            display_name: if e.display_name.is_empty() { e.id.clone() } else { e.display_name.clone() },
            active: active == Some(e.id.as_str()),
        })
        .collect()
}

fn active_name(manifest: &profiles::Manifest) -> String {
    manifest
        .last_active
        .as_deref()
        .and_then(|id| manifest.entries.iter().find(|e| e.id == id))
        .map(|e| if e.display_name.is_empty() { e.id.clone() } else { e.display_name.clone() })
        .unwrap_or_else(|| "无".into())
}

#[tauri::command(async)]
fn list_backups() -> Result<Vec<BackupInfo>, String> {
    let dir = switcher::backups_dir();
    let current = profiles::read(&profiles::base_dir()).ok();
    let current_ids: std::collections::HashSet<String> = current
        .as_ref()
        .map(|m| m.entries.iter().map(|e| e.id.clone()).collect())
        .unwrap_or_default();
    let current_active = current.as_ref().map(active_name).unwrap_or_else(|| "无".into());
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file = path.join(profiles::MANIFEST_FILE);
            if !file.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let modified = file
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let parsed = std::fs::read_to_string(&file)
                .ok()
                .and_then(|text| serde_json::from_str::<profiles::Manifest>(&text).ok());
            let (accounts, active, active_changed, added, removed, entries, parse_error) = match parsed {
                Some(manifest) => {
                    let entries = manifest_accounts(&manifest);
                    let ids: std::collections::HashSet<String> = manifest.entries.iter().map(|e| e.id.clone()).collect();
                    let added = manifest
                        .entries
                        .iter()
                        .filter(|e| !current_ids.contains(&e.id))
                        .map(|e| if e.display_name.is_empty() { e.id.clone() } else { e.display_name.clone() })
                        .collect();
                    let removed = current
                        .as_ref()
                        .map(|m| {
                            m.entries
                                .iter()
                                .filter(|e| !ids.contains(&e.id))
                                .map(|e| if e.display_name.is_empty() { e.id.clone() } else { e.display_name.clone() })
                                .collect()
                        })
                        .unwrap_or_default();
                    let active = active_name(&manifest);
                    let changed = manifest.last_active.as_deref() != current.as_ref().and_then(|m| m.last_active.as_deref());
                    (manifest.entries.len(), active, changed, added, removed, entries, None)
                }
                None => (0, "无法解析".into(), false, Vec::new(), Vec::new(), Vec::new(), Some("这份 profiles.json 解析失败".into())),
            };
            out.push(BackupInfo {
                name: name.to_string(),
                modified,
                accounts,
                active,
                active_changed,
                current_active: current_active.clone(),
                added,
                removed,
                entries,
                parse_error,
            });
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(out)
}

#[tauri::command(async)]
fn backup_now() -> Result<String, String> {
    let base = profiles::base_dir();
    if !base.join(profiles::MANIFEST_FILE).is_file() {
        return Err("Quick 的 profiles.json 不存在，先登录一次 Quick".into());
    }
    let path = switcher::backup_if_changed(&base, &switcher::backups_dir()).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command(async)]
fn delete_backup(name: String) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("备份名不合法".into());
    }
    let dir = switcher::backups_dir().join(&name);
    if !dir.is_dir() || !dir.join(profiles::MANIFEST_FILE).is_file() {
        return Err("找不到这份备份".into());
    }
    std::fs::remove_dir_all(dir).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn restore_backup(name: String) -> Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("备份名不合法".into());
    }
    if desktop::find().map_err(|e| e.to_string())?.is_some() {
        return Err("桌面端还开着，先退出 Quick 再恢复".into());
    }
    let src = switcher::backups_dir().join(&name).join(profiles::MANIFEST_FILE);
    if !src.is_file() {
        return Err("找不到这份备份".into());
    }
    switcher::restore_file(&profiles::base_dir(), &src).map_err(|e| e.to_string())
}

#[tauri::command(async)]
async fn list_cloud_sessions(account_id: String) -> Result<Vec<quick::cloud_sessions::CloudSession>, String> {
    quick::cloud_sessions::list(&profiles::base_dir(), &account_id).await.map_err(|e| e.to_string())
}

#[tauri::command(async)]
async fn export_cloud_sessions(account_id: String, ids: Vec<String>, format: String) -> Result<quick::cloud_sessions::CloudExport, String> {
    quick::cloud_sessions::export(&profiles::base_dir(), &account_id, &ids, &format).await.map_err(|e| e.to_string())
}

#[tauri::command(async)]
async fn import_cloud_sessions(
    source_account_id: String,
    target_account_id: String,
    ids: Vec<String>,
) -> Result<Vec<quick::cloud_sessions::CloudImport>, String> {
    quick::cloud_sessions::import_to_account(&source_account_id, &target_account_id, &ids)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn list_sessions(account_id: String) -> Result<Vec<quick::sessions::SessionInfo>, String> {
    quick::sessions::list(&profiles::base_dir(), &account_id).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn export_sessions(account_id: String, ids: Vec<String>, format: String) -> Result<quick::sessions::SessionExport, String> {
    quick::sessions::export(&profiles::base_dir(), &account_id, &ids, &format).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn device_overview() -> Result<device_ids::DeviceOverview, String> {
    let store = credstore::system();
    device_ids::overview(&profiles::base_dir(), &vault::dir(), store.as_ref()).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn replace_account_device(id: String) -> Result<device_ids::DeviceIdentity, String> {
    device_ids::replace_account(&profiles::base_dir(), &vault::dir(), &id).map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn restore_account_device(id: String, device_id: String, config: String, space: String) -> Result<(), String> {
    let target = vault::IdentityChange { device_id, config, space, replaced_at: String::new() };
    device_ids::restore_account(&profiles::base_dir(), &vault::dir(), &id, &target).map_err(|e| e.to_string())
}

/// Start the copy-and-paste login: returns the quick.aws.com sign-in address.
/// A local callback server also runs, so a browser on this machine can
/// finish both hops without any paste.
#[tauri::command(async)]
async fn manual_login_start() -> Result<String, String> {
    oauth::start_manual().await.map_err(|e| e.to_string())
}

/// Feed a pasted callback address into the same handler the local server uses.
#[tauri::command(async)]
fn manual_login_submit(pasted: String) -> Result<(), String> {
    oauth::submit_manual(&pasted).map_err(|e| e.to_string())
}

#[tauri::command(async)]
async fn manual_login_status() -> Result<oauth::ManualStatus, String> {
    oauth::manual_status().await.map_err(|e| e.to_string())
}

/// Copy one desktop account into the vault (the "local credentials" import).
#[tauri::command(async)]
fn import_desktop_account(id: String) -> Result<bool, String> {
    let store = credstore::system();
    vault::import_one(&profiles::base_dir(), store.as_ref(), &vault::dir(), &id).map_err(|e| e.to_string())
}

/// Remove an account from the vault, and optionally from the desktop.
#[tauri::command(async)]
fn delete_account(id: String, remove_desktop: bool, remove_data: bool) -> Result<(), String> {
    let store = credstore::system();
    manage::delete(
        &profiles::base_dir(),
        &switcher::backups_dir(),
        store.as_ref(),
        &vault::dir(),
        &id,
        remove_desktop,
        remove_data,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn account_details(id: String) -> Result<Option<accounts::AccountDetails>, String> {
    let store = credstore::system();
    accounts::details(&profiles::base_dir(), &vault::dir(), store.as_ref(), &id).map_err(|e| e.to_string())
}

/// Quota for one account, refreshing its access token when needed.
#[tauri::command(async)]
async fn account_usage(id: String) -> Result<api::Usage, String> {
    let original = {
        let store = credstore::system();
        vault::get_blob(store.as_ref(), &id)
            .map_err(|e| e.to_string())?
            .ok_or("账号库里没有这个账号的登录凭据，先同步桌面端")?
    };
    let (blob, usage) = match api::usage_for_blob(&api::client(), &original).await {
        Ok(v) => v,
        Err(e) => {
            if let api::ApiError::Auth(message) = &e {
                let flag = if message.contains("403") { "forbidden" } else { "auth_invalid" };
                let _ = vault::set_flag(&vault::dir(), &id, Some(flag));
            }
            return Err(e.to_string());
        }
    };
    if blob != original {
        let store = credstore::system();
        vault::upsert(&vault::dir(), store.as_ref(), &id, &blob).map_err(|e| e.to_string())?;
        // with the desktop closed, keep its copy in step too; otherwise the
        // next last_active sync would replace the vault's fresher token
        if matches!(desktop::find(), Ok(None)) {
            let store = credstore::system();
            let _ = switcher::write_desktop_blob(&profiles::base_dir(), store.as_ref(), &id, &blob);
        }
    }
    Ok(usage)
}

#[derive(serde::Serialize)]
struct AppInfo {
    version: String,
    base_dir: String,
    vault_dir: String,
    backups_dir: String,
}

#[tauri::command]
fn app_info(app: tauri::AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        base_dir: profiles::base_dir().display().to_string(),
        vault_dir: vault::dir().display().to_string(),
        backups_dir: switcher::backups_dir().display().to_string(),
    }
}

#[tauri::command]
async fn check_for_update(app: tauri::AppHandle) -> Result<updates::UpdateInfo, String> {
    updates::check(&app.package_info().version).await
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "打开主窗口", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "刷新账号", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &refresh, &sep, &quit])?;
    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "refresh" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                let _ = app.emit("tray://refresh", ());
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[tauri::command]
fn autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let al = app.autolaunch();
    if enabled {
        al.enable().map_err(|e| e.to_string())
    } else {
        al.disable().map_err(|e| e.to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .setup(|app| {
            setup_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            desktop_status,
            switch_account,
            force_quit_desktop,
            restore_last_backup,
            sync_from_desktop,
            remove_from_vault,
            login_account,
            list_sessions,
            export_sessions,
            list_cloud_sessions,
            export_cloud_sessions,
            import_cloud_sessions,
            device_overview,
            replace_account_device,
            restore_account_device,
            manual_login_start,
            manual_login_submit,
            manual_login_status,
            import_desktop_account,
            delete_account,
            account_details,
            account_usage,
            list_backups,
            backup_now,
            delete_backup,
            restore_backup,
            app_info,
            check_for_update,
            autostart_enabled,
            set_autostart
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
