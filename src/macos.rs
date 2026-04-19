use std::{
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use core_foundation::{
    base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, string::CFString,
};
use core_foundation_sys::{base::Boolean, dictionary::CFDictionaryRef, string::CFStringRef};

const PRIVACY_ACCESSIBILITY: &str = "Privacy_Accessibility";
const PRIVACY_AUTOMATION: &str = "Privacy_Automation";
const APP_REACTIVATION_DELAY: Duration = Duration::from_millis(120);

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> Boolean;
}

#[derive(Debug, Clone, Default)]
pub struct FrontmostApp {
    pub name: Option<String>,
    pub bundle_id: Option<String>,
    pub path: Option<String>,
}

impl FrontmostApp {
    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .or(self.bundle_id.as_deref())
            .or(self.path.as_deref())
            .unwrap_or("previous app")
    }
}

pub fn capture_frontmost_app() -> Result<Option<FrontmostApp>> {
    let output = run_capture("lsappinfo", &["front"])?;
    let Some(asn) = output
        .split_whitespace()
        .find(|token| token.starts_with("ASN:"))
        .map(str::to_owned)
    else {
        return Ok(None);
    };

    let app = FrontmostApp {
        name: lsappinfo_field(&asn, "name", "Name")?,
        bundle_id: lsappinfo_field(&asn, "bundleID", "CFBundleIdentifier")?,
        path: lsappinfo_field(&asn, "bundlepath", "LSBundlePath")?,
    };

    if app.name.is_none() && app.bundle_id.is_none() && app.path.is_none() {
        return Ok(None);
    }

    Ok(Some(app))
}

pub fn reactivate_previous_app(app: Option<&FrontmostApp>) -> Result<()> {
    let Some(app) = app else {
        return Ok(());
    };

    if let Some(bundle_id) = app.bundle_id.as_deref()
        && run_quiet("open", &["-b", bundle_id]).is_ok()
    {
        thread::sleep(APP_REACTIVATION_DELAY);
        return Ok(());
    }

    if let Some(path) = app.path.as_deref()
        && run_quiet("open", &["-a", path]).is_ok()
    {
        thread::sleep(APP_REACTIVATION_DELAY);
        return Ok(());
    }

    if let Some(name) = app.name.as_deref()
        && run_quiet("open", &["-a", name]).is_ok()
    {
        thread::sleep(APP_REACTIVATION_DELAY);
        return Ok(());
    }

    bail!("failed to reactivate {}", app.display_name());
}

pub fn ensure_accessibility_trusted(prompt: bool) -> bool {
    let prompt_key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let prompt_value = if prompt {
        CFBoolean::true_value()
    } else {
        CFBoolean::false_value()
    };
    let options = CFDictionary::from_CFType_pairs(&[(prompt_key, prompt_value)]);

    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0 }
}

pub fn open_accessibility_settings() {
    open_privacy_settings(PRIVACY_ACCESSIBILITY);
}

pub fn open_automation_settings() {
    open_privacy_settings(PRIVACY_AUTOMATION);
}

pub fn automation_denied(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("-1743")
        || lower.contains("not authorized to send apple events")
        || lower.contains("not authorised to send apple events")
}

pub fn accessibility_denied(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("assistive access")
        || lower.contains("accessibility")
        || lower.contains("not allowed")
}

fn open_privacy_settings(anchor: &str) {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
    let _ = run_quiet("open", &[url.as_str()]);
}

fn lsappinfo_field(asn: &str, selector: &str, key: &str) -> Result<Option<String>> {
    let output = run_capture("lsappinfo", &["info", "-only", selector, asn])?;
    Ok(parse_lsappinfo_field(&output, key))
}

fn parse_lsappinfo_field(output: &str, key: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (left, right) = line.split_once('=')?;
        if left.trim().trim_matches('"') != key {
            return None;
        }
        Some(right.trim().trim_matches('"').to_owned())
    })
}

fn run_capture(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run `{program}`"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.is_empty() {
            bail!("`{program}` exited with status {}", output.status);
        }
        bail!("{stderr}");
    }

    String::from_utf8(output.stdout).context("command output was not UTF-8")
}

fn run_quiet(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("failed to run `{program}`"))?;

    if status.success() {
        return Ok(());
    }

    bail!("`{program}` exited with status {status}");
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::parse_lsappinfo_field;

    #[test]
    fn parses_quoted_lsappinfo_fields() {
        let output = "\"LSBundlePath\"=\"/Applications/Alacritty.app\"\n";
        assert_eq!(
            parse_lsappinfo_field(output, "LSBundlePath").as_deref(),
            Some("/Applications/Alacritty.app")
        );
    }

    #[test]
    fn ignores_other_lsappinfo_fields() {
        let output = "\"Name\"=\"Alacritty\"\n";
        assert_eq!(parse_lsappinfo_field(output, "LSBundlePath"), None);
    }

    #[test]
    fn prefers_full_path_display_if_name_missing() {
        let app = super::FrontmostApp {
            name: None,
            bundle_id: None,
            path: Some("/Applications/Alacritty.app".to_owned()),
        };
        assert_eq!(app.display_name(), "/Applications/Alacritty.app");
        assert_eq!(
            Path::new(app.display_name())
                .file_name()
                .and_then(|value| value.to_str()),
            Some("Alacritty.app")
        );
    }
}
