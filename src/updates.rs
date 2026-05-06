//! Check for newer releases on GitHub.

use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tracing::warn;

const GITHUB_RELEASES_URL: &str = "https://api.github.com/repos/sloppish/runx/releases/latest";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

pub fn check_for_updates() {
    match fetch_latest_release() {
        Ok(release) => {
            let latest = release
                .tag_name
                .strip_prefix('v')
                .unwrap_or(&release.tag_name);
            if is_newer(latest, CURRENT_VERSION) {
                show_update_available(latest, &release.html_url);
            } else {
                show_up_to_date();
            }
        }
        Err(error) => {
            warn!(error = %format!("{error:#}"), "update check failed");
            show_error(&format!("{error:#}"));
        }
    }
}

fn fetch_latest_release() -> Result<GitHubRelease> {
    let output = Command::new("curl")
        .args([
            "-s",
            "-w",
            "\n%{http_code}",
            "-H",
            "Accept: application/vnd.github+json",
            GITHUB_RELEASES_URL,
        ])
        .output()
        .context("failed to run curl")?;

    if !output.status.success() {
        bail!("curl exited with status {}", output.status);
    }

    let raw = String::from_utf8(output.stdout).context("GitHub API returned non-UTF-8 response")?;

    let (body, status_line) = raw
        .rsplit_once('\n')
        .context("unexpected curl output format")?;

    let status: u16 = status_line
        .trim()
        .parse()
        .context("failed to parse HTTP status code")?;

    if status == 404 {
        bail!("no releases published yet");
    }
    if status != 200 {
        bail!("GitHub API returned HTTP {status}");
    }

    serde_json::from_str(body).context("failed to parse GitHub release response")
}

fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let l = parse(latest);
    let c = parse(current);
    l > c
}

fn show_update_available(version: &str, url: &str) {
    let script = format!(
        r#"display dialog "A new version of Runx is available: v{version}\n\nYou are running v{current}." buttons {{"Later", "View Release"}} default button 2 with title "Runx Update Available" with icon caution"#,
        version = version,
        current = CURRENT_VERSION,
    );
    let result = Command::new("osascript").arg("-e").arg(&script).output();
    if let Ok(output) = result {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("View Release") {
            let _ = Command::new("open").arg(url).spawn();
        }
    }
}

fn show_up_to_date() {
    let script = format!(
        r#"display dialog "You're on the latest version (v{})." buttons {{"OK"}} default button 1 with title "Runx" with icon note"#,
        CURRENT_VERSION,
    );
    let _ = Command::new("osascript").arg("-e").arg(&script).output();
}

fn show_error(message: &str) {
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        r#"display dialog "Could not check for updates:\n\n{escaped}" buttons {{"OK"}} default button 1 with title "Runx" with icon stop"#,
    );
    let _ = Command::new("osascript").arg("-e").arg(&script).output();
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn detects_newer_version() {
        assert!(is_newer("0.1.0", "0.0.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.0.2", "0.0.1"));
    }

    #[test]
    fn detects_same_or_older_version() {
        assert!(!is_newer("0.0.0", "0.0.0"));
        assert!(!is_newer("0.0.1", "0.0.2"));
        assert!(!is_newer("1.0.0", "1.0.0"));
    }
}
