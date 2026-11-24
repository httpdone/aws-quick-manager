//! Errors as the UI receives them.
//!
//! Every command fails with an [`AppError`]: a stable `code` the frontend
//! translates, the `params` that go into the sentence, and the backend's own
//! Chinese wording in `message` as a fallback for codes the UI doesn't know
//! (and for the `aqm` command line). The UI decides what to do by `code`,
//! never by matching `message`.
//!
//! Codes are `area.what`, e.g. `desktop.still_running`. `params.detail`,
//! where present, is raw text from the OS, a server or a library and is shown
//! as is in either language.

use crate::quick::api::ApiError;
use crate::quick::cloud_sessions::CloudError;
use crate::quick::credstore::CredError;
use crate::quick::desktop::DesktopError;
use crate::quick::device_ids::DeviceIdError;
use crate::quick::manage::ManageError;
use crate::quick::oauth::LoginError;
use crate::quick::sessions::SessionError;
use crate::quick::switcher::SwitchError;
use crate::quick::vault::VaultError;
use serde::Serialize;
use serde_json::{Map, Value};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppError {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
    pub message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, params: Map::new(), message: message.into() }
    }

    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.params.insert(key.to_string(), value.into());
        self
    }

    /// Something that shouldn't happen (a panicked task, a poisoned lock).
    pub fn internal(e: impl fmt::Display) -> Self {
        let detail = e.to_string();
        Self::new("internal", format!("内部错误：{detail}")).with("detail", detail)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

fn detail(code: &'static str, message: String, detail: impl Into<Value>) -> AppError {
    AppError::new(code, message).with("detail", detail)
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        let text = e.to_string();
        detail("io.failed", format!("读写文件失败：{text}"), text)
    }
}

impl From<DesktopError> for AppError {
    fn from(e: DesktopError) -> Self {
        let message = e.to_string();
        match e {
            DesktopError::Probe(d) => detail("desktop.probe_failed", message, d),
            DesktopError::StillRunning(seconds, remaining) => {
                AppError::new("desktop.still_running", message).with("seconds", seconds).with("remaining", remaining)
            }
            DesktopError::CloseRefused(code) => AppError::new("desktop.close_refused", message).with("code", code),
            DesktopError::BackendStillRunning(seconds) => {
                AppError::new("desktop.backend_still_running", message).with("seconds", seconds)
            }
            DesktopError::Launch(d) => detail("desktop.launch_failed", message, d),
            DesktopError::NotInstalled(searched) => AppError::new("desktop.not_installed", message).with("searched", searched),
            DesktopError::NoMainExe(dir) => AppError::new("desktop.no_main_exe", message).with("dir", dir),
            DesktopError::AmbiguousExe(dir, names) => {
                AppError::new("desktop.ambiguous_exe", message).with("dir", dir).with("names", names)
            }
            DesktopError::ForceKill(d) => detail("desktop.force_kill_failed", message, d),
        }
    }
}

impl From<CredError> for AppError {
    fn from(e: CredError) -> Self {
        let message = e.to_string();
        match e {
            CredError::Os(d) => detail("credentials.os", message, d),
            CredError::Corrupt(target) => AppError::new("credentials.corrupt", message).with("target", target),
            CredError::Unsupported => AppError::new("credentials.unsupported", message),
        }
    }
}

impl From<VaultError> for AppError {
    fn from(e: VaultError) -> Self {
        let message = e.to_string();
        match e {
            VaultError::Io(io) => detail("vault.io_failed", message, io.to_string()),
            VaultError::Cred(c) => c.into(),
        }
    }
}

impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        let message = e.to_string();
        match e {
            ApiError::Http(d) => detail("api.http", message, d),
            ApiError::Auth(d) => detail("api.auth", message, d),
        }
    }
}

impl From<SwitchError> for AppError {
    fn from(e: SwitchError) -> Self {
        let message = e.to_string();
        match e {
            SwitchError::NoSuchAccount(id) => AppError::new("account.not_found", message).with("id", id),
            SwitchError::AlreadyActive => AppError::new("switch.already_active", message),
            SwitchError::NoBackup => AppError::new("backup.none", message),
            SwitchError::Busy => AppError::new("desktop.busy", message),
            SwitchError::DesktopRunning => AppError::new("desktop.running", message),
            SwitchError::NoCredentials(id) => AppError::new("account.no_credentials", message).with("id", id),
            SwitchError::RefreshFailed(d) => detail("switch.refresh_failed", message, d),
            SwitchError::Cred(d) => detail("credentials.write_failed", message, d),
            SwitchError::VerifyFailed(backup) => AppError::new("switch.verify_failed", message).with("backup", backup),
            SwitchError::Device(d) => detail("device.io_failed", message, d),
            SwitchError::DeviceConflictWith(account) => AppError::new("device.conflict_with", message).with("account", account),
            SwitchError::DeviceInUse(id) => AppError::new("device.in_use", message).with("id", id),
            SwitchError::Desktop(d) => d.into(),
            SwitchError::Io(io) => io.into(),
        }
    }
}

impl From<DeviceIdError> for AppError {
    fn from(e: DeviceIdError) -> Self {
        let message = e.to_string();
        match e {
            DeviceIdError::DesktopRunning => AppError::new("desktop.running", message),
            DeviceIdError::NoAccount(id) => AppError::new("account.not_found", message).with("id", id),
            DeviceIdError::NotInHistory => AppError::new("device.not_in_history", message),
            DeviceIdError::Conflict(id) => AppError::new("device.in_use", message).with("id", id),
            DeviceIdError::ConfigConflict(id) => AppError::new("device.config_in_use", message).with("id", id),
            DeviceIdError::ConfigDrift(account) => AppError::new("device.config_drift", message).with("account", account),
            DeviceIdError::Io(io) => detail("device.io_failed", message, io.to_string()),
            DeviceIdError::Desktop(d) => d.into(),
        }
    }
}

impl From<ManageError> for AppError {
    fn from(e: ManageError) -> Self {
        let message = e.to_string();
        match e {
            ManageError::DesktopRunning => AppError::new("desktop.running", message),
            ManageError::Desktop(d) => d.into(),
            ManageError::Vault(v) => v.into(),
            ManageError::Cred(c) => c.into(),
            ManageError::Io(io) => io.into(),
        }
    }
}

impl From<SessionError> for AppError {
    fn from(e: SessionError) -> Self {
        let message = e.to_string();
        match e {
            SessionError::NoAccount(id) => AppError::new("account.not_found", message).with("id", id),
            SessionError::NoProfile => AppError::new("account.no_profile", message),
            SessionError::Db(d) => detail("sessions.db_failed", message, d),
            SessionError::NothingToExport => AppError::new("sessions.nothing_to_export", message),
            SessionError::Io(io) => detail("export.write_failed", message, io.to_string()),
        }
    }
}

impl From<CloudError> for AppError {
    fn from(e: CloudError) -> Self {
        let message = e.to_string();
        match e {
            CloudError::NoAccount(id) => AppError::new("account.not_found", message).with("id", id),
            CloudError::NoProfile => AppError::new("account.no_profile", message),
            CloudError::NoAgent => AppError::new("cloud.no_agent", message),
            CloudError::Spawn(d) => detail("cloud.spawn_failed", message, d),
            CloudError::NotReady(d) => detail("cloud.not_ready", message, d),
            CloudError::Session(d) => detail("cloud.session_failed", message, d),
            CloudError::Api(d) => detail("cloud.api_failed", message, d),
            CloudError::NothingToExport => AppError::new("sessions.nothing_to_export", message),
            CloudError::NothingSelected => AppError::new("sessions.nothing_selected", message),
            CloudError::NoEvents(id) => AppError::new("sessions.no_events", message).with("id", id),
            CloudError::NothingUploaded => AppError::new("sessions.nothing_uploaded", message),
            CloudError::Io(io) => detail("export.write_failed", message, io.to_string()),
        }
    }
}

impl From<LoginError> for AppError {
    fn from(e: LoginError) -> Self {
        let message = e.to_string();
        match e {
            LoginError::Io(io) => detail("login.port_failed", message, io.to_string()),
            LoginError::Browser(d) => detail("login.browser_failed", message, d),
            LoginError::Timeout => AppError::new("login.timeout", message),
            LoginError::Exists(account) => AppError::new("login.exists", message).with("account", account),
            LoginError::NoSession => AppError::new("login.no_session", message),
            LoginError::SessionExpired => AppError::new("login.session_expired", message),
            LoginError::BadPaste => AppError::new("login.bad_paste", message),
            LoginError::StateMismatch => AppError::new("login.state_mismatch", message),
            LoginError::StaleLink => AppError::new("login.stale_link", message),
            LoginError::OutOfOrder => AppError::new("login.out_of_order", message),
            LoginError::Denied(d) => detail("login.denied", message, d),
            LoginError::MissingInCallback(field) => AppError::new("login.bad_callback", message).with("field", field),
            LoginError::MissingInResponse(field) => AppError::new("login.bad_response", message).with("field", field),
            LoginError::TokenExchange(d) => detail("login.token_exchange", message, d),
            LoginError::UserInfo(d) => detail("login.user_info", message, d),
            LoginError::RegisterClient(d) => detail("login.register_client", message, d),
            LoginError::StateBroken => AppError::new("login.state_broken", message),
            LoginError::SessionEnded => AppError::new("login.session_ended", message),
            LoginError::WrongAccount(account) => AppError::new("login.wrong_account", message).with("account", account),
            LoginError::Internal(d) => detail("internal", message, d),
            LoginError::Vault(v) => v.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_code_params_and_message() {
        let e: AppError = DesktopError::StillRunning(20, "PID 1 Quick.exe".into()).into();
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "desktop.still_running");
        assert_eq!(v["params"]["seconds"], 20);
        assert_eq!(v["params"]["remaining"], "PID 1 Quick.exe");
        assert!(v["message"].as_str().unwrap().contains("20"));
    }

    #[test]
    fn empty_params_are_left_out() {
        let v = serde_json::to_value(AppError::from(SwitchError::AlreadyActive)).unwrap();
        assert_eq!(v["code"], "switch.already_active");
        assert!(v.get("params").is_none());
    }

    #[test]
    fn wrapped_errors_keep_the_inner_code() {
        let e: AppError = SwitchError::Desktop(DesktopError::CloseRefused(128)).into();
        assert_eq!(e.code, "desktop.close_refused");
        assert_eq!(e.params["code"], 128);
    }
}
