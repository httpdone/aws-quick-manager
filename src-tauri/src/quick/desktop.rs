//! Find, quit and start the Amazon Quick desktop app.
//!
//! What is known (from the magpie-aws-quick project, which reverse-engineered
//! `app.asar` and runs against an installed desktop):
//! - Windows installs per-user to `%LOCALAPPDATA%\Programs\quickwork\`
//!   (`Program Files\quickwork` also checked); the app ships
//!   `resources\agent.zip` there.
//! - macOS installs to `/Applications/Amazon Quick.app`.
//! - The Electron main process spawns a frozen Python backend,
//!   `quickwork-agent(.exe)`, extracted into the app's userData folder, so it
//!   lives *outside* the install folder and is a child of the main process.
//! - magpie can also start `quickwork-agent --outpost-only` on its own; that
//!   one is not the desktop and must not be treated as it.
//!
//! So the desktop is recognised by *where its executable lives* (inside the
//! install folder), not by guessing a process or product name. The main
//! executable's file name isn't recorded anywhere, so it is read from the
//! install folder at run time.
//!
//! Not yet known: how the app handles a close request (quit, or hide to the
//! tray). Quitting is therefore a polite request first. On Windows that is
//! `taskkill` without `/F`, which posts WM_CLOSE to the main process's
//! windows and fails outright when it has none to close; that failure is
//! reported at once instead of waiting out the timeout. Either way the error
//! names the processes still running, and the UI offers a force quit.

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

pub const QUIT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long the Python backend may take to exit after its Electron parent.
pub const BACKEND_TIMEOUT: Duration = Duration::from_secs(30);
pub const BACKEND_EXE: &str = if cfg!(windows) { "quickwork-agent.exe" } else { "quickwork-agent" };

#[derive(Debug, thiserror::Error)]
pub enum DesktopError {
    #[error("查找桌面端进程失败：{0}")]
    Probe(String),
    #[error("Quick 收到退出请求后 {0} 秒内没有退出（仍在运行：{1}）")]
    StillRunning(u64, String),
    #[error("Quick 没有退出：普通退出请求被拒绝（taskkill 返回 {0}，表示它没有可关闭的窗口）")]
    CloseRefused(i32),
    #[error("Quick 的后台进程 {0} 秒内没有退出，先不要切号")]
    BackendStillRunning(u64),
    #[error("启动桌面端失败：{0}")]
    Launch(String),
    #[error("没找到桌面端安装位置（已查找：{0}）")]
    NotInstalled(String),
    #[error("强制结束 Quick 失败：{0}")]
    ForceKill(String),
}

/// One OS process. On macOS `exe` is the full path from `ps`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub exe: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub installed: bool,
    /// what `launch` would start
    pub exe: Option<String>,
    pub running: bool,
    pub pid: Option<u32>,
}

// ---------- where the app is installed ----------

/// Install folders to look in, most likely first.
pub fn install_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    #[cfg(windows)]
    {
        let env = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty()).map(PathBuf::from);
        if let Some(lad) = env("LOCALAPPDATA") {
            v.push(lad.join("Programs").join("quickwork"));
        }
        v.push(env("ProgramFiles").unwrap_or_else(|| PathBuf::from(r"C:\Program Files")).join("quickwork"));
        if let Some(pfx) = env("ProgramFiles(x86)") {
            v.push(pfx.join("quickwork"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        v.push(PathBuf::from("/Applications/Amazon Quick.app"));
        if let Some(h) = dirs::home_dir() {
            v.push(h.join("Applications").join("Amazon Quick.app"));
        }
    }
    v
}

/// The main executable in a Windows install folder: the one `.exe` at the
/// top level that isn't the uninstaller.
pub fn pick_main_exe(names: &[String]) -> Result<String, String> {
    let exes: Vec<&String> = names
        .iter()
        .filter(|n| n.to_lowercase().ends_with(".exe"))
        .filter(|n| !n.to_lowercase().starts_with("uninstall"))
        .collect();
    match exes.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err("安装目录里没有 .exe".into()),
        many => Err(format!("安装目录里有多个 .exe，无法确定主程序：{}", many.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

#[cfg(windows)]
fn main_exe_in(dir: &Path) -> Option<Result<PathBuf, String>> {
    let rd = std::fs::read_dir(dir).ok()?;
    let names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    Some(pick_main_exe(&names).map(|n| dir.join(n)))
}

/// The install folder and what to launch (Windows: the main `.exe`; macOS:
/// the `.app` bundle).
pub fn installation() -> Result<(PathBuf, PathBuf), DesktopError> {
    let dirs = install_dirs();
    #[cfg(windows)]
    for d in &dirs {
        match main_exe_in(d) {
            Some(Ok(exe)) => return Ok((d.clone(), exe)),
            Some(Err(why)) => return Err(DesktopError::NotInstalled(format!("{}：{why}", d.display()))),
            None => {}
        }
    }
    #[cfg(target_os = "macos")]
    for d in &dirs {
        if d.is_dir() {
            return Ok((d.clone(), d.clone()));
        }
    }
    Err(DesktopError::NotInstalled(dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join("；")))
}

pub fn installed_exe() -> Option<String> {
    installation().ok().map(|(_, exe)| exe.display().to_string())
}

// ---------- recognising the running app ----------

fn norm(p: &str) -> String {
    let p = p.replace('\\', "/");
    if cfg!(windows) { p.to_lowercase() } else { p }
}

/// A path inside `dir` (case-insensitive on Windows).
pub fn is_inside(path: &str, dir: &Path) -> bool {
    let d = norm(&dir.display().to_string());
    let d = d.trim_end_matches('/');
    let p = norm(path);
    !d.is_empty() && p.len() > d.len() && p.starts_with(d) && p.as_bytes()[d.len()] == b'/'
}

/// The app's main process: one running from the install folder whose parent
/// isn't (Electron helpers are its children; the Python backend lives in
/// userData and isn't matched at all).
pub fn pick_root(procs: &[Proc], install: &Path, own_pid: u32) -> Option<Proc> {
    let cands: Vec<&Proc> = procs.iter().filter(|p| p.pid != own_pid && is_inside(&p.exe, install)).collect();
    cands.iter().find(|p| !cands.iter().any(|c| c.pid == p.ppid)).map(|p| (*p).clone())
}

// ---------- listing processes ----------

/// `ConvertTo-Json` output: one object, an array of them, or nothing.
pub fn parse_ps_json(text: &str) -> Result<Vec<Proc>, DesktopError> {
    let text = text.trim().trim_start_matches('\u{feff}');
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| DesktopError::Probe(format!("无法解析进程列表：{e}")))?;
    let items = match v {
        Value::Array(a) => a,
        o @ Value::Object(_) => vec![o],
        _ => Vec::new(),
    };
    let num = |o: &Value, k: &str| o.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
    Ok(items
        .iter()
        .map(|o| Proc { pid: num(o, "pid"), ppid: num(o, "ppid"), exe: o.get("exe").and_then(Value::as_str).unwrap_or_default().to_string() })
        .filter(|p| p.pid != 0 && !p.exe.is_empty())
        .collect())
}

/// `ps -axo pid=,ppid=,comm=` output.
pub fn parse_ps_lines(text: &str) -> Vec<Proc> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, rest) = line.split_once(char::is_whitespace)?;
            let rest = rest.trim_start();
            let (ppid, comm) = rest.split_once(char::is_whitespace)?;
            Some(Proc { pid: pid.parse().ok()?, ppid: ppid.parse().ok()?, exe: comm.trim().to_string() })
        })
        .collect()
}

#[cfg(windows)]
fn hidden(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(windows)]
fn processes() -> Result<Vec<Proc>, DesktopError> {
    const PS: &str = r#"[Console]::OutputEncoding=[Text.Encoding]::UTF8
$ErrorActionPreference='SilentlyContinue'
@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath } | ForEach-Object {
  [pscustomobject]@{pid=[uint32]$_.ProcessId; ppid=[uint32]$_.ParentProcessId; exe=[string]$_.ExecutablePath}
}) | ConvertTo-Json -Compress"#;
    let out = hidden(Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", PS]))
        .output()
        .map_err(|e| DesktopError::Probe(e.to_string()))?;
    if !out.status.success() {
        return Err(DesktopError::Probe(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    parse_ps_json(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(target_os = "macos")]
fn processes() -> Result<Vec<Proc>, DesktopError> {
    let out = Command::new("ps").args(["-axo", "pid=,ppid=,comm="]).output().map_err(|e| DesktopError::Probe(e.to_string()))?;
    Ok(parse_ps_lines(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn processes() -> Result<Vec<Proc>, DesktopError> {
    Ok(Vec::new())
}

// ---------- the operations ----------

/// The running desktop, if it is installed and running.
pub fn find() -> Result<Option<Proc>, DesktopError> {
    let Ok((dir, _)) = installation() else { return Ok(None) };
    Ok(pick_root(&processes()?, &dir, std::process::id()))
}

pub fn status() -> Result<Status, DesktopError> {
    let exe = installed_exe();
    let p = find()?;
    Ok(Status { installed: exe.is_some(), exe, running: p.is_some(), pid: p.map(|p| p.pid) })
}

#[cfg(windows)]
fn request_quit(p: &Proc) -> Result<(), DesktopError> {
    // No /F: post WM_CLOSE to the main process's windows and let the app shut
    // its own children down (quit() waits for the backend). No /T either:
    // the helpers and the backend have no windows, so including them would
    // make taskkill report failure even when the main window took the request.
    let out = hidden(Command::new("taskkill").args(["/PID", &p.pid.to_string()]))
        .output()
        .map_err(|e| DesktopError::Probe(format!("taskkill: {e}")))?;
    if out.status.success() {
        return Ok(());
    }
    // the process may have exited between find() and taskkill
    if find()?.is_none() {
        return Ok(());
    }
    Err(DesktopError::CloseRefused(out.status.code().unwrap_or(-1)))
}

#[cfg(target_os = "macos")]
fn request_quit(_: &Proc) -> Result<(), DesktopError> {
    // a normal app quit, same as ⌘Q
    let _ = Command::new("osascript").args(["-e", r#"tell application "Amazon Quick" to quit"#]).output();
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn request_quit(_: &Proc) -> Result<(), DesktopError> {
    Ok(())
}

/// Ask the app to quit and wait until it and its Python backend are gone.
pub fn quit(p: &Proc, timeout: Duration) -> Result<(), DesktopError> {
    // capture children before the request: once the root exits they are
    // reparented and can no longer be found by parent id
    let backends = backend_pids(p.pid).unwrap_or_default();
    request_quit(p)?;
    let start = Instant::now();
    while start.elapsed() < timeout {
        sleep(Duration::from_millis(500));
        if find()?.is_none() {
            // the backend may still be writing session data or refreshing
            // tokens; writing credentials before it exits can be undone
            wait_backends(&backends, BACKEND_TIMEOUT)?;
            sleep(Duration::from_millis(300));
            return Ok(());
        }
    }
    Err(DesktopError::StillRunning(timeout.as_secs(), remaining()))
}

/// The processes from the install folder that are still running, for the
/// timeout message: what is actually holding the app open.
fn remaining() -> String {
    let Ok((dir, _)) = installation() else { return "未知".into() };
    let procs = processes().unwrap_or_default();
    let left: Vec<String> = procs
        .iter()
        .filter(|p| is_inside(&p.exe, &dir))
        .map(|p| {
            let name = Path::new(&p.exe).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.exe.clone());
            format!("PID {} {name}", p.pid)
        })
        .collect();
    if left.is_empty() { "无".into() } else { left.join("、") }
}

/// Force-kill the app and its backend, then wait for both to disappear.
/// Used only after a polite quit timed out and the user confirmed.
pub fn force_quit(p: &Proc) -> Result<(), DesktopError> {
    let backends = backend_pids(p.pid).unwrap_or_default();
    #[cfg(windows)]
    {
        let out = Command::new("taskkill")
            .args(["/PID", &p.pid.to_string(), "/T", "/F"])
            .output()
            .map_err(|e| DesktopError::ForceKill(e.to_string()))?;
        if !out.status.success() && find()?.is_some() {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(DesktopError::ForceKill(if msg.is_empty() { "taskkill failed".into() } else { msg }));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-9", &p.pid.to_string()]).status();
        for pid in &backends {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if find()?.is_none() {
            wait_backends(&backends, Duration::from_secs(10))?;
            return Ok(());
        }
        sleep(Duration::from_millis(200));
    }
    Err(DesktopError::ForceKill("进程仍在运行".into()))
}

/// PIDs of the app's Python backend, found among the root's descendants. A
/// `quickwork-agent --outpost-only` started by another tool (for example
/// magpie) isn't under the app and is deliberately left alone.
pub fn backend_pids(root: u32) -> Result<Vec<u32>, DesktopError> {
    let procs = processes()?;
    let mut pending = vec![root];
    let mut out = Vec::new();
    while let Some(parent) = pending.pop() {
        for p in procs.iter().filter(|p| p.ppid == parent) {
            pending.push(p.pid);
            if is_backend(&p.exe) {
                out.push(p.pid);
            }
        }
    }
    Ok(out)
}

fn is_backend(exe: &str) -> bool {
    Path::new(exe).file_name().is_some_and(|n| n.eq_ignore_ascii_case(BACKEND_EXE))
}

fn wait_backends(pids: &[u32], timeout: Duration) -> Result<(), DesktopError> {
    if pids.is_empty() {
        return Ok(());
    }
    let start = Instant::now();
    loop {
        if processes()?.iter().all(|p| !pids.contains(&p.pid)) {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            return Err(DesktopError::BackendStillRunning(timeout.as_secs()));
        }
        sleep(Duration::from_millis(300));
    }
}

#[cfg(windows)]
pub fn launch(exe: &str) -> Result<(), DesktopError> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let mut cmd = Command::new(exe);
    if let Some(dir) = Path::new(exe).parent() {
        cmd.current_dir(dir);
    }
    // the desktop sets QUICKWORK_ROOT itself from last_active
    cmd.env_remove("QUICKWORK_ROOT")
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
        .map_err(|e| DesktopError::Launch(e.to_string()))
}

#[cfg(target_os = "macos")]
pub fn launch(bundle: &str) -> Result<(), DesktopError> {
    let out = Command::new("open").arg(bundle).output().map_err(|e| DesktopError::Launch(e.to_string()))?;
    if out.status.success() { Ok(()) } else { Err(DesktopError::Launch(String::from_utf8_lossy(&out.stderr).trim().to_string())) }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn launch(_: &str) -> Result<(), DesktopError> {
    Err(DesktopError::Launch("只支持 Windows 和 macOS".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, ppid: u32, exe: &str) -> Proc {
        Proc { pid, ppid, exe: exe.into() }
    }

    #[test]
    fn main_exe_skips_uninstaller() {
        let names = vec!["Uninstall X.exe".to_string(), "X.exe".to_string(), "ffmpeg.dll".to_string()];
        assert_eq!(pick_main_exe(&names).unwrap(), "X.exe");
        assert!(pick_main_exe(&["a.dll".to_string()]).is_err());
        assert!(pick_main_exe(&["a.exe".to_string(), "b.exe".to_string()]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn inside_is_case_insensitive_and_needs_a_separator() {
        let dir = Path::new(r"C:\Users\u\AppData\Local\Programs\quickwork");
        assert!(is_inside(r"c:\users\U\appdata\local\programs\QUICKWORK\X.exe", dir));
        assert!(!is_inside(r"C:\Users\u\AppData\Local\Programs\quickwork-other\X.exe", dir));
        assert!(!is_inside(r"C:\Users\u\AppData\Local\Programs\quickwork", dir));
    }

    #[cfg(windows)]
    #[test]
    fn root_is_the_main_process_not_helpers_backend_or_magpie() {
        let dir = Path::new(r"C:\Users\u\AppData\Local\Programs\quickwork");
        let procs = vec![
            p(10, 1, r"C:\Users\u\AppData\Local\Programs\quickwork\X.exe"),
            p(11, 10, r"C:\Users\u\AppData\Local\Programs\quickwork\X.exe"),
            // the desktop's backend, extracted into userData
            p(12, 10, r"C:\Users\u\AppData\Roaming\quickwork\agent\quickwork-agent.exe"),
            // magpie's own outpost
            p(20, 19, r"C:\Users\u\AppData\Local\Temp\magpie-aws-quick\agent\1\quickwork-agent.exe"),
        ];
        assert_eq!(pick_root(&procs, dir, 0).unwrap().pid, 10);
        assert_eq!(pick_root(&procs[2..], dir, 0), None);
    }

    #[test]
    fn ps_json_single_array_and_empty() {
        let one = r#"{"pid":10,"ppid":1,"exe":"C:\\A\\X.exe"}"#;
        assert_eq!(parse_ps_json(one).unwrap().len(), 1);
        let many = format!("[{one},{{\"pid\":11,\"ppid\":10,\"exe\":\"C:\\\\A\\\\y.exe\"}}]");
        assert_eq!(parse_ps_json(&many).unwrap().len(), 2);
        assert!(parse_ps_json("  \r\n").unwrap().is_empty());
    }

    #[test]
    fn ps_lines_keep_full_path() {
        let text = "  501     1 /Applications/Amazon Quick.app/Contents/MacOS/Amazon Quick\n  502   501 /usr/bin/python3\n";
        let v = parse_ps_lines(text);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].exe, "/Applications/Amazon Quick.app/Contents/MacOS/Amazon Quick");
        assert!(is_inside(&v[0].exe, Path::new("/Applications/Amazon Quick.app")));
        assert!(!is_inside(&v[1].exe, Path::new("/Applications/Amazon Quick.app")));
    }

    /// Reads this machine; run with `cargo test -- --ignored live`.
    #[test]
    #[ignore]
    fn live_desktop_status() {
        println!("install dirs: {:?}", install_dirs());
        println!("installation: {:?}", installation());
        println!("status: {:?}", status());
    }
}
