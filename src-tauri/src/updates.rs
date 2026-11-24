//! Commit subjects between two releases, for the update dialog.
//!
//! Checking, downloading, signature verification and installing are all done
//! by Tauri's updater plugin (see `plugins.updater` in tauri.conf.json). This
//! only fills in "what changed" when a release has no notes of its own.

use crate::error::AppError;
use reqwest::{Client, StatusCode};
use semver::Version;
use serde::{Deserialize, de::DeserializeOwned};
use std::time::Duration;

const REPO_API: &str = "https://api.github.com/repos/httpdone/aws-quick-manager";
/// Most commits listed in the dialog.
const MAX_COMMITS: usize = 50;

#[derive(Deserialize)]
struct Compare {
    #[serde(default)]
    commits: Vec<CompareCommit>,
}

#[derive(Deserialize)]
struct CompareCommit {
    commit: CommitBody,
}

#[derive(Deserialize)]
struct CommitBody {
    message: String,
}

fn network_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        AppError::new("update.timeout", "连接 GitHub 超时，请稍后再试")
    } else {
        AppError::new("update.network", "无法连接 GitHub，请检查网络后重试")
    }
}

async fn get_json<T: DeserializeOwned>(http: &Client, url: &str) -> Result<T, AppError> {
    let response = http
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(network_error)?;
    match response.status() {
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
            return Err(AppError::new("update.rate_limited", "GitHub 限制了请求，请稍后再试"));
        }
        StatusCode::NOT_FOUND => return Err(AppError::new("update.no_release", "GitHub 上暂无正式发布版本")),
        status if !status.is_success() => {
            return Err(AppError::new("update.http", format!("GitHub 请求失败（HTTP {status}）")).with("status", status.as_u16()));
        }
        _ => {}
    }
    response.json::<T>().await.map_err(|error| {
        if error.is_decode() {
            AppError::new("update.invalid_response", "GitHub 返回了无效的版本信息")
        } else {
            network_error(error)
        }
    })
}

/// Accepts `1.2.3` or `v1.2.3`; anything else never reaches the URL.
fn parse_version(v: &str) -> Result<Version, AppError> {
    Version::parse(v.trim().strip_prefix('v').unwrap_or(v.trim()))
        .map_err(|_| AppError::new("update.invalid_version", "GitHub 返回了无效的版本号"))
}

/// Commit subjects, newest first, without merge commits.
fn commit_subjects(compare: Compare) -> Vec<String> {
    let mut out: Vec<String> = compare
        .commits
        .into_iter()
        .filter_map(|c| c.commit.message.lines().next().map(|l| l.trim().to_string()))
        .filter(|s| !s.is_empty() && !s.starts_with("Merge "))
        .collect();
    out.reverse();
    out.truncate(MAX_COMMITS);
    out
}

/// What was committed between release `v{current}` and release `v{latest}`.
pub(crate) async fn changes(current: &str, latest: &str) -> Result<Vec<String>, AppError> {
    let from = parse_version(current)?;
    let to = parse_version(latest)?;
    let http = Client::builder()
        .user_agent("AWS-Quick-Manager")
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(AppError::internal)?;
    let compare: Compare = get_json(&http, &format!("{REPO_API}/compare/v{from}...v{to}")).await?;
    Ok(commit_subjects(compare))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_with_or_without_v_are_accepted() {
        assert_eq!(parse_version("v1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(parse_version("1.2.3").unwrap(), Version::new(1, 2, 3));
    }

    #[test]
    fn anything_else_is_rejected() {
        for bad in ["latest", "1.2.3...main", "../x", ""] {
            assert_eq!(parse_version(bad).unwrap_err().code, "update.invalid_version", "{bad}");
        }
    }

    #[test]
    fn commit_subjects_are_newest_first_without_merges() {
        let c = |m: &str| CompareCommit { commit: CommitBody { message: m.into() } };
        let list = commit_subjects(Compare {
            commits: vec![c("Add screenshots\n\nbody"), c("Merge pull request #2"), c("Bump version to 0.1.1 (#6)")],
        });
        assert_eq!(list, vec!["Bump version to 0.1.1 (#6)", "Add screenshots"]);
    }
}
