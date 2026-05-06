use std::{ffi::c_int, thread, time::Duration};

use anyhow::{Result, bail};
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
