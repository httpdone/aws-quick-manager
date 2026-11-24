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
    #[error("没有可导出的会话")]
    NothingToExport,
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

/// Headings in exported Markdown, in the UI's language (`en`, else Chinese).
pub(crate) struct ExportLabels {
    pub session_id: &'static str,
    pub account: &'static str,
    pub updated: &'static str,
    pub messages: &'static str,
    pub synced: &'static str,
    pub yes: &'static str,
    pub local_only: &'static str,
    pub no_events: &'static str,
    pub untitled: &'static str,
}

pub(crate) fn export_labels(lang: &str) -> ExportLabels {
    if lang == "en" {
        ExportLabels {
            session_id: "Session ID",
            account: "Account",
            updated: "Updated",
            messages: "Messages",
            synced: "Synced to cloud",
            yes: "Yes",
            local_only: "No (local only)",
            no_events: "_No events for this session in the local database._",
            untitled: "Untitled session",
        }
    } else {
        ExportLabels {
            session_id: "会话 ID",
            account: "账号",
            updated: "更新时间",
            messages: "消息数",
            synced: "云端同步",
            yes: "是",
            local_only: "否（仅本地）",
            no_events: "_本地库里没有这个会话的事件。_",
            untitled: "未命名会话",
        }
    }
}

/// Unix seconds as RFC 3339, "—" when unknown.
pub(crate) fn fmt_secs(secs: f64) -> String {
    if secs > 0.0 {
        chrono::DateTime::from_timestamp(secs as i64, 0).map(|t| t.to_rfc3339()).unwrap_or_else(|| secs.to_string())
    } else {
        "—".into()
    }
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
        let mut info: SessionInfo = row.map_err(|e| SessionError::Db(e.to_string()))?;
        // an untitled session stays untitled; the UI shows its own label
        info.title = info.title.trim().to_string();
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
                if KEYS.contains(&key.as_str())
                    && let Value::String(s) = child
                {
                    let s = s.trim();
                    if !s.is_empty() {
                        out.push(s.chars().take(4000).collect());
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
    lang: &str,
) -> Result<SessionExport, SessionError> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
    export_to(&switcher::app_dir().join("exports").join(&stamp), base, account_id, ids, format, lang)
}

fn export_to(dir: &Path, base: &Path, account_id: &str, ids: &[String], format: &str, lang: &str) -> Result<SessionExport, SessionError> {
    let l = export_labels(lang);
    let all = list(base, account_id)?;
    let selected: Vec<SessionInfo> = if ids.is_empty() {
        all
    } else {
        all.into_iter().filter(|s| ids.iter().any(|id| id == &s.id)).collect()
    };
    if selected.is_empty() {
        return Err(SessionError::NothingToExport);
    }

    let path = db_path(base, account_id)?;
    let conn = open(&path)?;
    let markdown = format != "json";

    fs::create_dir_all(dir)?;

    let mut events_total = 0usize;
    let mut documents = Vec::new();
    let mut json_docs: Vec<Value> = Vec::new();
    for session in &selected {
        let events = read_events(&conn, &session.id)?;
        events_total += events.len();
        if markdown {
            let mut text = format!(
                "# {}\n\n- {}: {}\n- {}: {}\n- {}: {}\n- {}: {}\n- {}: {}\n\n",
                if session.title.is_empty() { l.untitled } else { &session.title },
                l.session_id,
                session.id,
                l.account,
                account_id,
                l.updated,
                fmt_secs(session.updated_at),
                l.messages,
                session.message_count,
                l.synced,
                if session.server_session_id.is_empty() { l.local_only } else { l.yes }
            );
            if events.is_empty() {
                text.push_str(l.no_events);
                text.push('\n');
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
            json_docs.push(json!({
                "account": account_id,
                "session": session,
                "events": events_json,
            }));
        }
    }

    // JSON: one object for one session, an array for several (a file of
    // concatenated objects isn't valid JSON)
    if !markdown {
        let text = if json_docs.len() == 1 {
            serde_json::to_string_pretty(&json_docs[0])
        } else {
            serde_json::to_string_pretty(&Value::Array(json_docs.clone()))
        }
        .map_err(|e| SessionError::Db(e.to_string()))?;
        let file = if selected.len() == 1 {
            let s = &selected[0];
            dir.join(format!("{}-{}.json", safe_name(&s.title), s.id.chars().take(8).collect::<String>()))
        } else {
            dir.join("sessions.json")
        };
        fs::write(&file, text)?;
        return Ok(SessionExport { path: file.display().to_string(), sessions: json_docs.len(), events: events_total });
    }

    if documents.len() == 1 {
        let (name, id, content) = &documents[0];
        let file = dir.join(format!("{name}-{}.md", id.chars().take(8).collect::<String>()));
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
        combined.push_str("\n\n---\n\n");
    }
    let file = dir.join("sessions.md");
    fs::write(&file, combined)?;
    Ok(SessionExport {
        path: file.display().to_string(),
        sessions: documents.len(),
        events: events_total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick::profiles::{Manifest, ProfileEntry};

    fn setup(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("aqm-sessions-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let base = root.join("base");
        let m = Manifest { version: 1, entries: vec![ProfileEntry { id: "social-a".into(), ..Default::default() }], ..Default::default() };
        profiles::write(&base, &m).unwrap();
        let db = base.join("profiles").join("social-a").join("sessions");
        fs::create_dir_all(&db).unwrap();
        let conn = Connection::open(db.join("sessions.db")).unwrap();
        conn.execute_batch(
            "create table sessions (id text, title text, created_at real, updated_at real, message_count integer, \
               server_session_id text, archived integer, deleted_at real);
             create table session_events (session_id text, type text, timestamp real, data text);
             insert into sessions values ('s-old', 'Old', 1, 10, 2, 'srv-1', 0, null);
             insert into sessions values ('s-new', '', 1, 20, 1, null, 1, null);
             insert into sessions values ('s-gone', 'Gone', 1, 30, 0, null, 0, 5);
             insert into session_events values ('s-old', 'message_user', 10, '{\"content\":\"hello\"}');
             insert into session_events values ('s-old', 'message_assistant_complete', 11, '{\"data\":{\"text\":\"hi there\"}}');",
        )
        .unwrap();
        root
    }

    #[test]
    fn list_skips_deleted_sessions_newest_first() {
        let root = setup("list");
        let list = list(&root.join("base"), "social-a").unwrap();
        assert_eq!(list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["s-new", "s-old"]);
        assert_eq!(list[0].title, "");
        assert!(list[0].archived && list[0].server_session_id.is_empty());
        assert_eq!(list[1].server_session_id, "srv-1");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_account_without_a_session_store_has_no_sessions() {
        let root = setup("none");
        fs::remove_dir_all(root.join("base").join("profiles")).unwrap();
        assert!(list(&root.join("base"), "social-a").unwrap().is_empty());
        assert!(matches!(list(&root.join("base"), "social-x"), Err(SessionError::NoAccount(_))));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn markdown_export_includes_event_text() {
        let root = setup("md");
        let out = export_to(&root.join("out"), &root.join("base"), "social-a", &["s-old".into()], "md", "en").unwrap();
        assert_eq!((out.sessions, out.events), (1, 2));
        let text = fs::read_to_string(&out.path).unwrap();
        assert!(text.starts_with("# Old"));
        assert!(text.contains("- Session ID: s-old") && text.contains("- Synced to cloud: Yes"));
        assert!(text.contains("hello") && text.contains("hi there"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn json_export_of_several_sessions_is_one_valid_array() {
        let root = setup("json");
        let out = export_to(&root.join("out"), &root.join("base"), "social-a", &[], "json", "en").unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&out.path).unwrap()).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
        assert_eq!(out.sessions, 2);

        let one = export_to(&root.join("out1"), &root.join("base"), "social-a", &["s-old".into()], "json", "en").unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&one.path).unwrap()).unwrap();
        assert_eq!(v["session"]["id"], "s-old");
        assert_eq!(v["events"][0]["data"]["content"], "hello");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn nothing_selected_is_an_error() {
        let root = setup("empty");
        assert!(matches!(
            export_to(&root.join("out"), &root.join("base"), "social-a", &["nope".into()], "md", "en"),
            Err(SessionError::NothingToExport)
        ));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(safe_name("a/b:c?"), "a-b-c-");
        assert_eq!(safe_name("  ..  "), "session");
        assert_eq!(safe_name(&"x".repeat(100)).len(), 60);
    }

    #[test]
    fn markdown_headings_follow_the_language() {
        let root = setup("lang");
        let zh = export_to(&root.join("zh"), &root.join("base"), "social-a", &["s-new".into()], "md", "zh").unwrap();
        let text = fs::read_to_string(&zh.path).unwrap();
        assert!(text.starts_with("# 未命名会话"));
        assert!(text.contains("- 会话 ID: s-new") && text.contains("否（仅本地）") && text.contains("本地库里没有这个会话的事件"));
        let en = export_to(&root.join("en"), &root.join("base"), "social-a", &["s-new".into()], "md", "en").unwrap();
        let text = fs::read_to_string(&en.path).unwrap();
        assert!(text.starts_with("# Untitled session") && text.contains("No (local only)"));
        let _ = fs::remove_dir_all(&root);
    }
}
