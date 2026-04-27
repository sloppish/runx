use std::{
    env,
    ffi::c_int,
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
    NSWorkspace,
};

use super::{process::run_quiet, types::FrontmostApp};

const APP_REACTIVATION_DELAY: Duration = Duration::from_millis(120);

#[derive(Clone, Copy)]
pub(super) enum AppActivationMode {
    Default,
    AllWindows,
}

/// Opens an application bundle path through Launch Services.
pub fn open_application(path: &str) -> Result<()> {
    run_quiet("open", &[path])
}

/// Opens a System Settings deep-link URL.
pub fn open_settings(url: &str) -> Result<()> {
    run_quiet("open", &[url])
}

/// Opens `runx-config` inside Terminal.
pub fn open_settings_editor() -> Result<()> {
    let shell_command = settings_editor_shell_command()?;
    let script_path = settings_editor_command_file(&shell_command)?;
    let status = Command::new("open")
        .arg("-a")
        .arg("Terminal")
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to launch Terminal for runx-config")?;
    if status.success() {
        return Ok(());
    }

    let _ = fs::remove_file(script_path);
    bail!("failed to launch Terminal for runx-config: {status}");
}

/// Captures the app currently considered frontmost by macOS.
pub fn capture_frontmost_app() -> Result<Option<FrontmostApp>> {
    let workspace = NSWorkspace::sharedWorkspace();
    let Some(app) = workspace.frontmostApplication() else {
        return Ok(None);
    };

    let app = FrontmostApp {
        name: app.localizedName().map(|value| value.to_string()),
        bundle_id: app.bundleIdentifier().map(|value| value.to_string()),
        path: app
            .bundleURL()
            .and_then(|url| url.path().map(|path| path.to_string()))
            .or_else(|| {
                app.executableURL()
                    .and_then(|url| url.path().map(|path| path.to_string()))
                    .and_then(|path| bundle_root_from_executable_path(&path))
            }),
    };

    if app.name.is_none() && app.bundle_id.is_none() && app.path.is_none() {
        return Ok(None);
    }

    Ok(Some(app))
}

/// Returns whether the app can normally own foreground windows.
pub fn running_application_is_regular(pid: i64) -> bool {
    let running_apps = NSWorkspace::sharedWorkspace().runningApplications();
    for index in 0..running_apps.count() {
        let candidate = running_apps.objectAtIndex(index);
        if i64::from(candidate.processIdentifier()) == pid {
            return candidate.activationPolicy() == NSApplicationActivationPolicy::Regular;
        }
    }
    false
}

pub(super) fn open_named_application(name: &str) -> Result<()> {
    run_quiet("open", &["-a", name])
}

pub(super) fn running_application_pid_by_name(name: &str) -> Option<c_int> {
    let running_apps = NSWorkspace::sharedWorkspace().runningApplications();
    for index in 0..running_apps.count() {
        let candidate = running_apps.objectAtIndex(index);
        if candidate
            .localizedName()
            .is_some_and(|actual| actual.to_string() == name)
        {
            return Some(candidate.processIdentifier());
        }
    }
    None
}

pub(super) fn activate_running_application_by_pid(pid: c_int) -> Result<()> {
    activate_running_application_by_pid_with_mode(pid, AppActivationMode::Default)
}

pub(super) fn activate_running_application_by_pid_with_mode(
    pid: c_int,
    mode: AppActivationMode,
) -> Result<()> {
    let running_apps = NSWorkspace::sharedWorkspace().runningApplications();
    for index in 0..running_apps.count() {
        let candidate = running_apps.objectAtIndex(index);
        if candidate.processIdentifier() == pid {
            activate_running_application(&candidate, mode)?;
            return Ok(());
        }
    }
    bail!("failed to find running application for pid {pid}")
}

fn activate_running_application(app: &NSRunningApplication, mode: AppActivationMode) -> Result<()> {
    if app.isHidden() {
        let _ = app.unhide();
    }

    let options = match mode {
        AppActivationMode::Default => NSApplicationActivationOptions(0),
        AppActivationMode::AllWindows => NSApplicationActivationOptions::ActivateAllWindows,
    };

    if app.activateWithOptions(options) {
        thread::sleep(APP_REACTIVATION_DELAY);
        return Ok(());
    }

    let label = app
        .localizedName()
        .map(|value| value.to_string())
        .or_else(|| app.bundleIdentifier().map(|value| value.to_string()))
        .unwrap_or_else(|| "application".to_owned());
    bail!("failed to activate {label}")
}

pub(super) fn bundle_root_from_executable_path(executable_path: &str) -> Option<String> {
    std::path::Path::new(executable_path)
        .ancestors()
        .find(|ancestor| {
            matches!(
                ancestor.extension().and_then(|value| value.to_str()),
                Some("app" | "appex" | "prefPane")
            )
        })
        .map(|ancestor| ancestor.to_string_lossy().into_owned())
}

fn settings_editor_shell_command() -> Result<String> {
    let executable = env::current_exe().context("failed to resolve current executable path")?;
    let command = resolve_settings_editor_command(&executable)
        .ok_or_else(|| anyhow::anyhow!("failed to locate a runnable `runx-config` helper"))?;
    Ok(command)
}

fn resolve_settings_editor_command(executable: &Path) -> Option<String> {
    sibling_settings_editor(executable)
        .map(|path| shell_quote(path.to_string_lossy().as_ref()))
        .or_else(|| development_cargo_command(executable))
}

fn sibling_settings_editor(executable: &Path) -> Option<PathBuf> {
    let sibling = executable.with_file_name("runx-config");
    sibling.is_file().then_some(sibling)
}

fn development_cargo_command(executable: &Path) -> Option<String> {
    let repo_root = executable
        .ancestors()
        .find(|ancestor| ancestor.join("Cargo.toml").is_file())?;
    Some(format!(
        "cd {} && cargo run --bin runx-config",
        shell_quote(repo_root.to_string_lossy().as_ref())
    ))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'"'"'"#))
}

fn settings_editor_command_file(command: &str) -> Result<PathBuf> {
    let mut file = tempfile::Builder::new()
        .prefix("runx-config-")
        .suffix(".command")
        .tempfile()
        .context("failed to create runx-config launch script")?;
    file.write_all(settings_editor_command_script(command).as_bytes())
        .context("failed to write runx-config launch script")?;

    let mut permissions = file
        .as_file()
        .metadata()
        .context("failed to inspect runx-config launch script")?
        .permissions();
    permissions.set_mode(0o700);
    file.as_file()
        .set_permissions(permissions)
        .context("failed to make runx-config launch script executable")?;

    let (_file, path) = file
        .keep()
        .context("failed to persist runx-config launch script")?;
    Ok(path)
}

fn settings_editor_command_script(command: &str) -> String {
    format!(
        r#"#!/bin/zsh
rm -f -- "$0"
clear
unset NO_COLOR
{command}
status=$?
if [ "$status" -ne 0 ]; then
  printf '\nrunx-config exited with status %s. Press Return to continue.' "$status"
  read -r _
fi
exec "${{SHELL:-/bin/zsh}}" -l
"#
    )
}

#[cfg(test)]
mod tests {
    use super::{
        development_cargo_command, resolve_settings_editor_command, settings_editor_command_script,
    };

    #[test]
    fn resolves_sibling_runx_config_binary() {
        let root = tempfile::tempdir().expect("tempdir");
        let executable_dir = root.path().join("Contents/MacOS");
        std::fs::create_dir_all(&executable_dir).expect("create executable dir");
        let runx = executable_dir.join("runx");
        let runx_config = executable_dir.join("runx-config");
        std::fs::write(&runx, "").expect("write runx");
        std::fs::write(&runx_config, "").expect("write runx-config");

        let command = resolve_settings_editor_command(&runx).expect("resolved command");
        assert_eq!(command, format!("'{}'", runx_config.to_string_lossy()));
    }

    #[test]
    fn falls_back_to_cargo_run_for_development_tree() {
        let root = tempfile::tempdir().expect("tempdir");
        let repo = root.path().join("repo");
        let target_dir = repo.join("target/debug");
        std::fs::create_dir_all(&target_dir).expect("create target dir");
        std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"runx\"\n")
            .expect("write cargo toml");
        let runx = target_dir.join("runx");
        std::fs::write(&runx, "").expect("write runx");

        let command = development_cargo_command(&runx).expect("cargo fallback");
        assert_eq!(
            command,
            format!(
                "cd '{}' && cargo run --bin runx-config",
                repo.to_string_lossy()
            )
        );
    }

    #[test]
    fn settings_editor_script_runs_command_without_apple_events() {
        let script =
            settings_editor_command_script("'/Applications/Runx.app/Contents/MacOS/runx-config'");

        assert!(script.starts_with("#!/bin/zsh\n"));
        assert!(script.contains(
            "clear\nunset NO_COLOR\n'/Applications/Runx.app/Contents/MacOS/runx-config'\n"
        ));
        assert!(script.contains("exec \"${SHELL:-/bin/zsh}\" -l"));
    }
}
