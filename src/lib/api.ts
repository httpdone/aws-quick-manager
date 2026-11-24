import { invoke } from "@tauri-apps/api/core";
import type { AppError } from "./errors";

export interface Account {
  id: string;
  provider: string;
  display_name: string;
  email: string;
  role: string;
  aws_account_id: string;
  active: boolean;
  has_credentials: boolean;
  access_expires_at: number;
  refresh_expires_at: number;
  refresh_failing_since: unknown;
  last_used_at: string | null;
  in_vault: boolean;
  on_desktop: boolean;
  error: string | null;
  flag: string | null;
}

export interface DesktopStatus {
  installed: boolean;
  exe: string | null;
  running: boolean;
  pid: number | null;
}

export interface SwitchReport {
  backup: string;
  was_running: boolean;
  launched: boolean;
  /** things to tell the user after a switch that otherwise worked */
  notes: AppError[];
}

export interface ImportReport {
  imported: string[];
  updated: string[];
  unchanged: string[];
  /** [account id, reason] */
  skipped: [string, string][];
}

export interface UsageWindow {
  name: string;
  used_hours: number;
  limit_hours: number;
  left_hours: number;
}

export interface Usage {
  plan: string;
  trial_ends_at: number;
  windows: UsageWindow[];
}

export interface AccountDetails {
  id: string;
  provider: string;
  display_name: string;
  email: string;
  role: string;
  aws_account_id: string;
  tenant_url: string;
  client_id: string;
  issuer_url: string;
  created_at: string | null;
  last_used_at: string | null;
  added_at: string | null;
  last_synced_at: string | null;
  device_id: string;
  access_expires_at: number;
  refresh_expires_at: number;
  active: boolean;
  in_vault: boolean;
  on_desktop: boolean;
  flag: string | null;
  error: string | null;
}

export interface IdentityChange {
  device_id: string;
  config: string;
  space: string;
  replaced_at: string;
}

export interface DeviceIdentity {
  device_id: string;
  config: string;
  space: string;
}

export interface DeviceAccount {
  id: string;
  display_name: string;
  email: string;
  active: boolean;
  on_desktop: boolean;
  profile_device_id: string | null;
  vault_device_id: string;
  device_id: string;
  config: string;
  space: string;
  space_matches_config: boolean;
  matches: boolean;
  history: IdentityChange[];
}

export interface BaseIdentity {
  config: string;
  space: string;
  owner: string | null;
}

export interface DeviceOverview {
  accounts: DeviceAccount[];
  base: BaseIdentity;
  conflicts: IdConflict[];
}

/** An id more than one account uses. */
export interface IdConflict {
  kind: "device" | "config" | "space";
  id: string;
}

export interface SessionInfo {
  id: string;
  title: string;
  created_at: number;
  updated_at: number;
  message_count: number;
  server_session_id: string;
  archived: boolean;
}

export interface CloudSession {
  id: string;
  title: string;
  created_at: number;
  updated_at: number;
  message_count: number;
  archived: boolean;
}

export interface CloudExport {
  path: string;
  sessions: number;
  messages: number;
  /** sessions whose messages may be cut off at the 100-message limit */
  truncated: string[];
}

export interface CloudImport {
  source_session_id: string;
  target_session_id: string;
  events: number;
  messages: number;
}

export interface SessionExport {
  path: string;
  sessions: number;
  events: number;
}

export interface LoginOutcome {
  id: string;
  email: string;
  display_name: string;
}

export interface BackupAccount {
  id: string;
  display_name: string;
  active: boolean;
}

export interface BackupInfo {
  name: string;
  modified: number;
  accounts: number;
  active: string;
  active_changed: boolean;
  current_active: string;
  added: string[];
  removed: string[];
  entries: BackupAccount[];
  /** "invalid_manifest" when the backup can't be parsed */
  parse_error: string | null;
}

export interface AppInfo {
  version: string;
  base_dir: string;
  vault_dir: string;
  backups_dir: string;
}


export const listAccounts = () => invoke<Account[]>("list_accounts");
export const desktopStatus = () => invoke<DesktopStatus>("desktop_status");
export const forceQuitDesktop = () => invoke<boolean>("force_quit_desktop");
export const switchAccount = (id: string, launch = true) => invoke<SwitchReport>("switch_account", { id, launch });
export const restoreLastBackup = () => invoke<string>("restore_last_backup");
export const syncFromDesktop = () => invoke<ImportReport>("sync_from_desktop");
export const removeFromVault = (id: string) => invoke<void>("remove_from_vault", { id });
export interface ManualStatus {
  step: string;
  url: string | null;
  account: LoginOutcome | null;
  error: AppError | null;
}

export const loginAccount = () => invoke<LoginOutcome>("login_account");
/** Sign in to an existing account again and replace its stored sign-in. */
export const reloginAccount = (id: string) => invoke<LoginOutcome>("relogin_account", { id });
export const listSessions = (accountId: string) => invoke<SessionInfo[]>("list_sessions", { accountId });
/** `lang` picks the language of the Markdown headings. */
export const exportSessions = (accountId: string, ids: string[], format: "md" | "json", lang: "zh" | "en") =>
  invoke<SessionExport>("export_sessions", { accountId, ids, format, lang });
export const listCloudSessions = (accountId: string) => invoke<CloudSession[]>("list_cloud_sessions", { accountId });
export const exportCloudSessions = (accountId: string, ids: string[], format: "md" | "json", lang: "zh" | "en") =>
  invoke<CloudExport>("export_cloud_sessions", { accountId, ids, format, lang });
export const importCloudSessions = (sourceAccountId: string, targetAccountId: string, ids: string[]) =>
  invoke<CloudImport[]>("import_cloud_sessions", { sourceAccountId, targetAccountId, ids });
export const manualLoginStart = () => invoke<string>("manual_login_start");
export const manualLoginSubmit = (pasted: string) => invoke<void>("manual_login_submit", { pasted });
export const manualLoginStatus = () => invoke<ManualStatus>("manual_login_status");
export const importDesktopAccount = (id: string) => invoke<boolean>("import_desktop_account", { id });
export const deleteAccount = (id: string, removeDesktop: boolean, removeData: boolean) =>
  invoke<void>("delete_account", { id, removeDesktop, removeData });
export const accountDetails = (id: string) => invoke<AccountDetails | null>("account_details", { id });
export const accountUsage = (id: string) => invoke<Usage>("account_usage", { id });
export const deviceOverview = () => invoke<DeviceOverview>("device_overview");
export const replaceAccountDevice = (id: string) => invoke<DeviceIdentity>("replace_account_device", { id });
export const restoreAccountDevice = (id: string, identity: IdentityChange) =>
  invoke<void>("restore_account_device", {
    id,
    deviceId: identity.device_id,
    config: identity.config,
    space: identity.space,
  });
export const listBackups = () => invoke<BackupInfo[]>("list_backups");
export const backupNow = () => invoke<string>("backup_now");
export const deleteBackup = (name: string) => invoke<void>("delete_backup", { name });
export const restoreBackup = (name: string) => invoke<void>("restore_backup", { name });
export const appInfo = () => invoke<AppInfo>("app_info");
/** Commit subjects between two releases (for an update without notes). */
export const updateChanges = (current: string, latest: string) => invoke<string[]>("update_changes", { current, latest });
export const autostartEnabled = () => invoke<boolean>("autostart_enabled");
export const setAutostart = (enabled: boolean) => invoke<void>("set_autostart", { enabled });
export const setTrayLanguage = (lang: "zh" | "en") => invoke<void>("set_tray_language", { lang });
