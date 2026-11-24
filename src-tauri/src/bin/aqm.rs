//! Small command-line front end for AWS Quick Manager.
//!
//!   aqm list
//!   aqm current
//!   aqm switch <index|email|id> [--no-launch]
//!
//! It uses the same account store, token refresh and switch transaction as
//! the GUI; nothing is written unless `switch` commits.

use aws_quick_manager_lib::quick::{accounts, credstore, profiles, switcher, vault};

fn usage() -> ! {
    eprintln!("usage: aqm list | current | switch <index|email|id> [--no-launch]");
    std::process::exit(2);
}

fn account_list() -> Result<Vec<accounts::AccountView>, String> {
    let store = credstore::system();
    accounts::list(&profiles::base_dir(), &vault::dir(), store.as_ref()).map_err(|e| e.to_string())
}

fn print_list(list: &[accounts::AccountView]) {
    if list.is_empty() {
        println!("no accounts");
        return;
    }
    for (i, a) in list.iter().enumerate() {
        let mark = if a.active { "*" } else { " " };
        let vault = if a.in_vault { "vault" } else { "" };
        let desktop = if a.on_desktop { "desktop" } else { "" };
        println!(
            "{mark} {:>2}. {:<24} {:<32} {:<8} {}",
            i + 1,
            a.display_name,
            a.email,
            [vault, desktop].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(","),
            a.id
        );
    }
}

fn find_account(list: &[accounts::AccountView], needle: &str) -> Result<accounts::AccountView, String> {
    if let Ok(index) = needle.parse::<usize>() {
        if index == 0 || index > list.len() {
            return Err(format!("序号超出范围：{index}"));
        }
        return Ok(list[index - 1].clone());
    }
    let needle = needle.to_lowercase();
    let matches: Vec<accounts::AccountView> = list
        .iter()
        .filter(|a| {
            a.id.to_lowercase() == needle
                || a.email.to_lowercase() == needle
                || a.display_name.to_lowercase() == needle
        })
        .cloned()
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("没有找到账号：{needle}")),
        many => Err(format!("匹配到 {} 个账号，请用 id：{}", many.len(), many.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("list") => account_list().map(|list| print_list(&list)),
        Some("current") => profiles::read(&profiles::base_dir())
            .map_err(|e| e.to_string())
            .map(|m| {
                let name = m
                    .last_active
                    .as_deref()
                    .and_then(|id| m.entries.iter().find(|e| e.id == id))
                    .map(|e| if e.display_name.is_empty() { e.id.clone() } else { e.display_name.clone() })
                    .unwrap_or_else(|| "none".into());
                println!("{name}");
            }),
        Some("switch") => {
            let Some(target) = args.get(1) else { usage() };
            let launch = !args.iter().any(|a| a == "--no-launch");
            let base = profiles::base_dir();
            let vault_dir = vault::dir();
            async {
                let _guard = switcher::lock_desktop_writes().map_err(|e| e.to_string())?;
                let list = account_list()?;
                let account = find_account(&list, target)?;
                let blob = switcher::refresh_target(&base, &account.id, &vault_dir)
                    .await
                    .map_err(|e| e.to_string())?;
                let store = credstore::system();
                let report = switcher::commit(&base, &switcher::backups_dir(), &account.id, launch, store.as_ref(), &vault_dir, &blob)
                    .map_err(|e| e.to_string())?;
                println!("switched to {} ({})", account.display_name, account.email);
                println!("backup: {}", report.backup);
                for note in &report.notes {
                    eprintln!("note: {}", note.message);
                }
                Ok(())
            }
            .await
        }
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("aqm: {e}");
        std::process::exit(1);
    }
}
