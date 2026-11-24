//! Small command-line front end for AWS Quick Manager.
//!
//!   aqm list
//!   aqm current
//!   aqm switch <index|email|id> [--no-launch]
//!
//! It uses the same account store, token refresh and switch transaction as
//! the GUI; nothing is written unless `switch` commits.
//!
//! Output is in Chinese or English: `AQM_LANG`, then `LC_ALL`, `LC_MESSAGES`
//! and `LANG`, then the system UI language decide.

use aws_quick_manager_lib::error::AppError;
use aws_quick_manager_lib::quick::{accounts, credstore, profiles, switcher, vault};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Lang {
    Zh,
    En,
}

/// The first variable with a real locale wins (`C` / `POSIX` say nothing);
/// otherwise the system UI language, otherwise English.
fn pick_lang(env: &[Option<String>], system_zh: Option<bool>) -> Lang {
    for v in env.iter().flatten() {
        let v = v.trim().to_ascii_lowercase();
        if v.is_empty() || v == "c" || v == "posix" || v.starts_with("c.") {
            continue;
        }
        return if v.starts_with("zh") { Lang::Zh } else { Lang::En };
    }
    if system_zh == Some(true) { Lang::Zh } else { Lang::En }
}

#[cfg(windows)]
fn system_zh() -> Option<bool> {
    // LANGID; the low 10 bits are the primary language, 0x04 is Chinese
    #[allow(unused_unsafe)]
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    Some(id & 0x3ff == 0x04)
}

#[cfg(not(windows))]
fn system_zh() -> Option<bool> {
    None
}

struct Out(Lang);

impl Out {
    fn s<'a>(&self, zh: &'a str, en: &'a str) -> &'a str {
        if self.0 == Lang::Zh { zh } else { en }
    }

    /// Backend messages are Chinese; in English show the code and its
    /// values instead, which name the same thing.
    fn err(&self, e: &AppError) -> String {
        if self.0 == Lang::Zh || e.code.starts_with("cli.") {
            return e.message.clone();
        }
        if e.params.is_empty() {
            return e.code.to_string();
        }
        let params: Vec<String> = e
            .params
            .iter()
            .map(|(k, v)| match v {
                serde_json::Value::String(s) => format!("{k}={s}"),
                other => format!("{k}={other}"),
            })
            .collect();
        format!("{} ({})", e.code, params.join(", "))
    }
}

fn usage(out: &Out) -> ! {
    eprintln!("{}", out.s("用法：aqm list | current | switch <序号|邮箱|id> [--no-launch]", "usage: aqm list | current | switch <index|email|id> [--no-launch]"));
    std::process::exit(2);
}

fn account_list() -> Result<Vec<accounts::AccountView>, AppError> {
    let store = credstore::system();
    Ok(accounts::list(&profiles::base_dir(), &vault::dir(), store.as_ref())?)
}

fn print_list(out: &Out, list: &[accounts::AccountView]) {
    if list.is_empty() {
        println!("{}", out.s("没有账号", "no accounts"));
        return;
    }
    for (i, a) in list.iter().enumerate() {
        let mark = if a.active { "*" } else { " " };
        let mut where_ = Vec::new();
        if a.in_vault {
            where_.push(out.s("账号库", "vault"));
        }
        if a.on_desktop {
            where_.push("Quick");
        }
        println!("{mark} {:>2}. {:<24} {:<32} {:<10} {}", i + 1, a.display_name, a.email, where_.join(","), a.id);
    }
}

fn find_account(out: &Out, list: &[accounts::AccountView], needle: &str) -> Result<accounts::AccountView, AppError> {
    if let Ok(index) = needle.parse::<usize>() {
        if index == 0 || index > list.len() {
            let text = if out.0 == Lang::Zh { format!("序号超出范围：{index}") } else { format!("no account number {index}") };
            return Err(AppError::new("cli.index_out_of_range", text));
        }
        return Ok(list[index - 1].clone());
    }
    let needle = needle.to_lowercase();
    let matches: Vec<accounts::AccountView> = list
        .iter()
        .filter(|a| a.id.to_lowercase() == needle || a.email.to_lowercase() == needle || a.display_name.to_lowercase() == needle)
        .cloned()
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => {
            let text = if out.0 == Lang::Zh { format!("没有找到账号：{needle}") } else { format!("no account matches {needle}") };
            Err(AppError::new("cli.not_found", text))
        }
        many => {
            let ids = many.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(", ");
            let text = if out.0 == Lang::Zh {
                format!("匹配到 {} 个账号，请用 id：{ids}", many.len())
            } else {
                format!("{} accounts match, use an id: {ids}", many.len())
            };
            Err(AppError::new("cli.ambiguous", text))
        }
    }
}

#[tokio::main]
async fn main() {
    let env: Vec<Option<String>> = ["AQM_LANG", "LC_ALL", "LC_MESSAGES", "LANG"].iter().map(|k| std::env::var(k).ok()).collect();
    let out = Out(pick_lang(&env, system_zh()));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result: Result<(), AppError> = match args.first().map(String::as_str) {
        Some("list") => account_list().map(|list| print_list(&out, &list)),
        Some("current") => profiles::read(&profiles::base_dir()).map_err(AppError::from).map(|m| {
            let name = m
                .last_active
                .as_deref()
                .and_then(|id| m.entries.iter().find(|e| e.id == id))
                .map(|e| if e.display_name.is_empty() { e.id.clone() } else { e.display_name.clone() })
                .unwrap_or_else(|| out.s("无", "none").to_string());
            println!("{name}");
        }),
        Some("switch") => {
            let Some(target) = args.get(1) else { usage(&out) };
            let launch = !args.iter().any(|a| a == "--no-launch");
            let base = profiles::base_dir();
            let vault_dir = vault::dir();
            async {
                let _guard = switcher::lock_desktop_writes()?;
                let list = account_list()?;
                let account = find_account(&out, &list, target)?;
                let blob = switcher::refresh_target(&base, &account.id, &vault_dir).await?;
                let store = credstore::system();
                let report = switcher::commit(&base, &switcher::backups_dir(), &account.id, launch, store.as_ref(), &vault_dir, &blob)?;
                if out.0 == Lang::Zh {
                    println!("已切换到 {}（{}）", account.display_name, account.email);
                    println!("清单备份：{}", report.backup);
                } else {
                    println!("switched to {} ({})", account.display_name, account.email);
                    println!("backup: {}", report.backup);
                }
                for note in &report.notes {
                    eprintln!("{}: {}", out.s("提示", "note"), out.err(note));
                }
                Ok::<(), AppError>(())
            }
            .await
        }
        _ => usage(&out),
    };
    if let Err(e) = result {
        eprintln!("aqm: {}", out.err(&e));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(xs: &[Option<&str>]) -> Vec<Option<String>> {
        xs.iter().map(|x| x.map(str::to_string)).collect()
    }

    #[test]
    fn language_comes_from_the_first_real_locale() {
        assert_eq!(pick_lang(&env(&[Some("en"), None, None, Some("zh_CN.UTF-8")]), Some(true)), Lang::En);
        assert_eq!(pick_lang(&env(&[None, None, None, Some("zh_CN.UTF-8")]), None), Lang::Zh);
        assert_eq!(pick_lang(&env(&[None, Some("C.UTF-8"), Some("POSIX"), Some("en_US.UTF-8")]), Some(true)), Lang::En);
        assert_eq!(pick_lang(&env(&[None, None, None, None]), Some(true)), Lang::Zh);
        assert_eq!(pick_lang(&env(&[None, None, None, None]), None), Lang::En);
    }

    #[test]
    fn english_errors_show_the_code_and_values() {
        let e = AppError::new("desktop.still_running", "中文").with("seconds", 20);
        assert_eq!(Out(Lang::En).err(&e), "desktop.still_running (seconds=20)");
        assert_eq!(Out(Lang::Zh).err(&e), "中文");
        let e = AppError::new("desktop.busy", "x").with("detail", "locked");
        assert_eq!(Out(Lang::En).err(&e), "desktop.busy (detail=locked)");
        assert_eq!(Out(Lang::En).err(&AppError::new("switch.already_active", "x")), "switch.already_active");
        assert_eq!(Out(Lang::En).err(&AppError::new("cli.not_found", "no account matches a")), "no account matches a");
    }
}
