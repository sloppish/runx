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

use crate::debug_log;

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

pub fn open_application(path: &str) -> Result<()> {
    run_quiet("open", &[path])
}

pub fn open_path(path: &str) -> Result<()> {
    run_quiet("open", &[path])
}

pub fn open_settings(url: &str) -> Result<()> {
    run_quiet("open", &[url])
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

pub fn focus_window(app_name: &str, window_title: &str) -> Result<Option<String>> {
    let script = r#"
on run argv
    set targetApp to item 1 of argv
    set targetWindow to item 2 of argv
    tell application "System Events"
        tell application process targetApp
            set frontmost to true
            try
                perform action "AXRaise" of first window whose name is targetWindow
            end try
        end tell
    end tell
end run
"#;

    let output = run_output("osascript", &["-e", script, "--", app_name, window_title])
        .with_context(|| format!("failed to raise window {window_title}"))?;

    if output.status.success() {
        return Ok(Some(format!("Focused {}", window_title)));
    }

    open_named_application(app_name).with_context(|| {
        format!("failed to focus window {window_title} and also failed to activate {app_name}")
    })?;

    if output.stderr.is_empty() {
        Ok(Some(format!(
            "Activated {} (window focus requires Accessibility permission)",
            app_name
        )))
    } else {
        Ok(Some(format!(
            "Activated {} (direct window focus unavailable: {})",
            app_name, output.stderr
        )))
    }
}

pub fn type_text_into_previous_app(
    text: &str,
    previous_app: Option<&FrontmostApp>,
) -> Result<String> {
    debug_log::append(format!(
        "type_text_into_previous_app start len={} previous_app={:?}",
        text.chars().count(),
        previous_app
    ));
    if !ensure_accessibility_trusted(true) {
        debug_log::append("type_text_into_previous_app accessibility preflight returned false");
        open_accessibility_settings();
        bail!(
            "Runx needs Accessibility permission to type into other apps. Approve the system prompt or enable your terminal/runx in System Settings > Privacy & Security > Accessibility, then retry."
        );
    }

    reactivate_previous_app(previous_app)?;
    debug_log::append("type_text_into_previous_app reactivated previous app");

    let script = r#"
on run argv
    tell application "System Events"
        keystroke item 1 of argv
    end tell
end run
"#;
    let output = run_output("osascript", &["-e", script, "--", text])
        .context("failed to launch osascript for text typing")?;
    debug_log::append(format!(
        "type_text_into_previous_app osascript status={} stderr={:?}",
        output.status, output.stderr
    ));

    if output.status.success() {
        return Ok("Typed into the previous app".to_owned());
    }

    if automation_denied(&output.stderr) {
        open_automation_settings();
        bail!(
            "macOS blocked Apple Events to System Events. Allow your terminal/runx under System Settings > Privacy & Security > Automation, then retry."
        );
    }

    if accessibility_denied(&output.stderr) {
        open_accessibility_settings();
        bail!(
            "macOS blocked assistive access while typing. Enable your terminal/runx in Privacy & Security > Accessibility, then retry."
        );
    }

    if output.stderr.is_empty() {
        bail!("typing failed for an unknown macOS reason");
    }

    bail!("{}", output.stderr);
}

fn reactivate_previous_app(app: Option<&FrontmostApp>) -> Result<()> {
    let Some(app) = app else {
        debug_log::append("reactivate_previous_app skipped: no previous app");
        return Ok(());
    };

    if let Some(bundle_id) = app.bundle_id.as_deref()
        && run_quiet("open", &["-b", bundle_id]).is_ok()
    {
        debug_log::append(format!(
            "reactivate_previous_app via bundle_id {:?}",
            bundle_id
        ));
        thread::sleep(APP_REACTIVATION_DELAY);
        return Ok(());
    }

    if let Some(path) = app.path.as_deref()
        && run_quiet("open", &["-a", path]).is_ok()
    {
        debug_log::append(format!("reactivate_previous_app via path {:?}", path));
        thread::sleep(APP_REACTIVATION_DELAY);
        return Ok(());
    }

    if let Some(name) = app.name.as_deref()
        && run_quiet("open", &["-a", name]).is_ok()
    {
        debug_log::append(format!("reactivate_previous_app via name {:?}", name));
        thread::sleep(APP_REACTIVATION_DELAY);
        return Ok(());
    }

    debug_log::append(format!(
        "reactivate_previous_app failed for name={:?} bundle_id={:?} path={:?}",
        app.name, app.bundle_id, app.path
    ));
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

fn open_accessibility_settings() {
    open_privacy_settings(PRIVACY_ACCESSIBILITY);
}

fn open_automation_settings() {
    open_privacy_settings(PRIVACY_AUTOMATION);
}

fn automation_denied(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("-1743")
        || lower.contains("not authorized to send apple events")
        || lower.contains("not authorised to send apple events")
}

fn accessibility_denied(stderr: &str) -> bool {
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

fn open_named_application(name: &str) -> Result<()> {
    run_quiet("open", &["-a", name])
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
    let output = run_output(program, args)?;

    if !output.status.success() {
        if output.stderr.is_empty() {
            bail!("`{program}` exited with status {}", output.status);
        }
        bail!("{}", output.stderr);
    }

    Ok(output.stdout)
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

struct CommandOutput {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_output(program: &str, args: &[&str]) -> Result<CommandOutput> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run `{program}`"))?;

    Ok(CommandOutput {
        status: output.status,
        stdout: String::from_utf8(output.stdout).context("command output was not UTF-8")?,
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
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
