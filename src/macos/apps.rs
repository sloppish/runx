use std::ffi::c_int;

use anyhow::{Result, bail};
use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSRunningApplication,
    NSWorkspace,
};

use super::{
    process::run_quiet,
    types::{FrontmostApp, RunningApp},
};

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

/// Returns user-visible running applications.
pub fn running_applications() -> Vec<RunningApp> {
    let running_apps = NSWorkspace::sharedWorkspace().runningApplications();
    let mut output = Vec::new();

    for index in 0..running_apps.count() {
        let app = running_apps.objectAtIndex(index);
        if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
            continue;
        }

        let name = app
            .localizedName()
            .map(|value| value.to_string())
            .or_else(|| app.bundleIdentifier().map(|value| value.to_string()))
            .unwrap_or_else(|| format!("pid {}", app.processIdentifier()));
        output.push(RunningApp {
            pid: i64::from(app.processIdentifier()),
            name,
            bundle_id: app.bundleIdentifier().map(|value| value.to_string()),
            path: app
                .bundleURL()
                .and_then(|url| url.path().map(|path| path.to_string()))
                .or_else(|| {
                    app.executableURL()
                        .and_then(|url| url.path().map(|path| path.to_string()))
                        .and_then(|path| bundle_root_from_executable_path(&path))
                }),
        });
    }

    output
}

/// Returns whether the app can normally own foreground windows.
/// Returns the subset of `pids` that belong to regular-activation-policy apps,
/// fetching the running application list only once.
pub fn regular_pids_from_running_applications(
    pids: &std::collections::HashSet<i64>,
) -> std::collections::HashSet<i64> {
    let running_apps = NSWorkspace::sharedWorkspace().runningApplications();
    let mut result = std::collections::HashSet::new();
    for index in 0..running_apps.count() {
        let app = running_apps.objectAtIndex(index);
        let app_pid = i64::from(app.processIdentifier());
        if pids.contains(&app_pid)
            && app.activationPolicy() == NSApplicationActivationPolicy::Regular
        {
            result.insert(app_pid);
        }
    }
    result
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
