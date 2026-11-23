//! Session browsing and export.
//!
//! Quick keeps each profile's session index and event cache in
//! `profiles/<id>/sessions/sessions.db`. The authoritative transcript lives in
//! the account's cloud SessionStore; this module exports what is in the local
//! database, read-only, and never writes to Quick's files.
//!
//! A row with `server_session_id` has been synced from the cloud; rows without
//! one are local-only. Exporting does not delete, migrate or re-upload them.

use super::device;
use super::profiles;
use super::switcher;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("没有这个账号：{0}")]
    NoAccount(String),
    #[error("这个账号还没有 Quick 数据目录，先在 Quick 里登录一次")]
    NoProfile,
    #[error("读取会话库失败：{0}")]
    Db(String),
    #[error("写导出文件失败：{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub message_count: i64,
    /// empty when the session has not been synced from the cloud
    pub server_session_id: String,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionExport {
    pub path: String,
    pub sessions: usize,
    pub events: usize,
}

fn db_path(base: &Path, account_id: &str) -> Result<PathBuf, SessionError> {
    let manifest = profiles::read(base).map_err(|e| SessionError::Db(e.to_string()))?;
    let Some(entry) = manifest.entries.iter().find(|e| e.id == account_id) else {
        return Err(SessionError::NoAccount(account_id.to_string()));
    };
    let dir = device::profile_dir(base, entry).join("sessions").join("sessions.db");
    if !dir.is_file() {
        return Err(SessionError::NoProfile);
    }
    Ok(dir)
}

fn open(path: &Path) -> Result<Connection, SessionError> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| SessionError::Db(e.to_string()))
}

pub fn list(base: &Path, account_id: &str) -> Result<Vec<SessionInfo>, SessionError> {
    let path = match db_path(base, account_id) {
        Ok(p) => p,
        Err(SessionError::NoProfile) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let conn = open(&path)?;
    let mut stmt = conn
        .prepare(
            "select id, coalesce(title, ''), coalesce(created_at, 0), coalesce(updated_at, 0), \
             coalesce(message_count, 0), coalesce(server_session_id, ''), coalesce(archived, 0) \
             from sessions where deleted_at is null order by updated_at desc",
        )
        .map_err(|e| SessionError::Db(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SessionInfo {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
                message_count: row.get(4)?,
                server_session_id: row.get(5)?,
                archived: row.get::<_, i64>(6)? != 0,
            })
        })
        .map_err(|e| SessionError::Db(e.to_string()))?;
    let mut out = Vec::new();
    for row in rows {
        let mut info = row.map_err(|e| SessionError::Db(e.to_string()))?;
        if info.title.trim().is_empty() {
            info.title = "未命名会话".into();
        }
        out.push(info);
    }
    Ok(out)
}

fn safe_name(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "session".into()
    } else {
        cleaned.chars().take(60).collect()
    }
}

fn collect_text(value: &Value, out: &mut Vec<String>) {
    const KEYS: [&str; 12] = [
        "text", "content", "delta", "message", "prompt", "title", "summary", "output", "result", "answer",
        "reasoning", "error",
    ];
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if KEYS.contains(&key.as_str()) {
                    if let Value::String(s) = child {
                        let s = s.trim();
                        if !s.is_empty() {
                            out.push(s.chars().take(4000).collect());
                        }
                    }
                }
                collect_text(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_text(v, out)),
        _ => {}
    }
}

fn event_text(data: &str) -> String {
    let value: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return data.chars().take(4000).collect(),
    };
    let mut texts = Vec::new();
    collect_text(&value, &mut texts);
    if texts.is_empty() {
        serde_json::to_string(&value)
            .map(|s| s.chars().take(4000).collect())
            .unwrap_or_default()
    } else {
        texts.join("\n\n")
    }
}

fn read_events(conn: &Connection, id: &str) -> Result<Vec<(String, f64, String)>, SessionError> {
    let mut stmt = conn
        .prepare("select type, coalesce(timestamp, 0), data from session_events where session_id = ?1 order by rowid")
        .map_err(|e| SessionError::Db(e.to_string()))?;
    let rows = stmt
        .query_map([id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| SessionError::Db(e.to_string()))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| SessionError::Db(e.to_string()))?);
    }
    Ok(out)
}

pub fn export(
    base: &Path,
    account_id: &str,
    ids: &[String],
    format: &str,
) -> Result<SessionExport, SessionError> {
    let all = list(base, account_id)?;
    let selected: Vec<SessionInfo> = if ids.is_empty() {
        all
    } else {
        all.into_iter().filter(|s| ids.iter().any(|id| id == &s.id)).collect()
    };
    if selected.is_empty() {
        return Err(SessionError::Db("没有可导出的会话".into()));
    }

    let path = db_path(base, account_id)?;
    let conn = open(&path)?;
    let markdown = format != "json";

    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let dir = switcher::app_dir().join("exports").join(&stamp);
    fs::create_dir_all(&dir)?;

    let mut events_total = 0usize;
    let mut documents = Vec::new();
    for session in &selected {
        let events = read_events(&conn, &session.id)?;
        events_total += events.len();
        if markdown {
            let mut text = format!(
                "# {}\n\n- 会话 ID: {}\n- 账号: {}\n- 更新时间: {}\n- 消息数: {}\n- 云端同步: {}\n\n",
                session.title,
                session.id,
                account_id,
                if session.updated_at > 0.0 {
                    chrono::DateTime::from_timestamp(session.updated_at as i64, 0)
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_else(|| session.updated_at.to_string())
                } else {
                    "—".into()
                },
                session.message_count,
                if session.server_session_id.is_empty() { "否（仅本地）" } else { "是" }
            );
            if events.is_empty() {
                text.push_str("_本地库里没有这个会话的事件。_\n");
            }
            for (kind, ts, data) in &events {
                let time = chrono::DateTime::from_timestamp(*ts as i64, 0)
                    .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| ts.to_string());
                text.push_str(&format!("## {} · {}\n\n{}\n\n", kind, time, event_text(data)));
            }
            documents.push((safe_name(&session.title), session.id.clone(), text));
        } else {
            let events_json: Vec<Value> = events
                .iter()
                .map(|(kind, ts, data)| {
                    json!({
                        "type": kind,
                        "timestamp": ts,
                        "data": serde_json::from_str::<Value>(data).unwrap_or_else(|_| Value::String(data.clone())),
                    })
                })
                .collect();
            documents.push((
                safe_name(&session.title),
                session.id.clone(),
                serde_json::to_string_pretty(&json!({
                    "account": account_id,
                    "session": session,
                    "events": events_json,
                }))
                .unwrap_or_default(),
            ));
        }
    }

    if documents.len() == 1 {
        let (name, id, content) = &documents[0];
        let file = dir.join(format!("{name}-{}.{}", &id[..id.len().min(8)], if markdown { "md" } else { "json" }));
        fs::write(&file, content)?;
        return Ok(SessionExport {
            path: file.display().to_string(),
            sessions: 1,
            events: events_total,
        });
    }

    let mut combined = String::new();
    for (_, _, content) in &documents {
        combined.push_str(content);
        if markdown {
            combined.push_str("\n\n---\n\n");
        } else {
            combined.push('\n');
        }
    }
    let file = dir.join(if markdown { "sessions.md" } else { "sessions.json" });
    fs::write(&file, combined)?;
    Ok(SessionExport {
        path: file.display().to_string(),
        sessions: documents.len(),
        events: events_total,
    })
}
