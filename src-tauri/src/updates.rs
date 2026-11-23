use reqwest::{Client, StatusCode};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, time::Duration};

const LATEST_RELEASE: &str =
    "https://api.github.com/repos/httpdone/aws-quick-manager/releases/latest";
const RELEASE_TAGS: &str = "https://github.com/httpdone/aws-quick-manager/releases/tag";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct UpdateInfo {
    current: String,
    latest: String,
    update_available: bool,
    release_url: String,
}

pub(crate) async fn check(current: &Version) -> Result<UpdateInfo, String> {
    let client = Client::builder()
        .user_agent("AWS-Quick-Manager")
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| "无法初始化网络连接")?;
    let response = client
        .get(LATEST_RELEASE)
        .send()
        .await
        .map_err(network_error)?;

    match response.status() {
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
            return Err("GitHub 限制了请求，请稍后再试".into());
        }
        StatusCode::NOT_FOUND => return Err("GitHub 上暂无正式发布版本".into()),
        status if !status.is_success() => return Err(format!("GitHub 请求失败（HTTP {status}）")),
        _ => {}
    }

    let release = response.json::<Release>().await.map_err(|error| {
        if error.is_decode() {
            "GitHub 返回了无效的版本信息"
        } else {
            network_error(error)
        }
    })?;
    compare_release(current, &release.tag_name)
}

fn network_error(error: reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "连接 GitHub 超时，请稍后再试"
    } else {
        "无法连接 GitHub，请检查网络后重试"
    }
}

fn compare_release(current: &Version, tag: &str) -> Result<UpdateInfo, String> {
    let latest = Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .map_err(|_| "GitHub 返回了无效的版本号")?;
    Ok(UpdateInfo {
        current: current.to_string(),
        latest: latest.to_string(),
        update_available: latest.cmp_precedence(current) == Ordering::Greater,
        release_url: format!("{RELEASE_TAGS}/{tag}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_release_versions_by_semver_precedence() {
        for (current, tag, expected) in [
            ("1.2.9", "v1.2.10", true),
            ("1.2.10", "v1.2.9", false),
            ("1.2.3", "1.2.3", false),
            ("1.2.3", "v1.2.3+build.2", false),
            ("1.2.3-rc.1", "v1.2.3", true),
            ("1.2.3", "v1.2.3-rc.1", false),
        ] {
            let result = compare_release(&Version::parse(current).unwrap(), tag).unwrap();
            assert_eq!(result.update_available, expected, "{current} vs {tag}");
            assert_eq!(result.current, current);
            assert_eq!(result.latest, tag.trim_start_matches('v'));
            assert_eq!(result.release_url, format!("{RELEASE_TAGS}/{tag}"));
        }
    }

    #[test]
    fn rejects_invalid_release_tags() {
        assert!(compare_release(&Version::new(1, 0, 0), "latest").is_err());
    }
}
