use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use objc2_foundation::{NSError, NSOperatingSystemVersion, NSProcessInfo};
use objc2_service_management::{
    SMAppService, SMAppServiceStatus, kSMErrorAlreadyRegistered, kSMErrorJobNotFound,
};
use plist::{Dictionary, Value};

use super::apps::bundle_root_from_executable_path;

const LOGIN_AGENT_PLIST_NAME: &str = "io.github.sloppish.runx.login-item.plist";
pub(super) const LOGIN_AGENT_LABEL: &str = "io.github.sloppish.runx.login-item";

#[derive(Debug, Clone)]
pub(super) struct LoginItemTarget {
    pub(super) executable_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmLoginItemStatus {
    Enabled,
    Disabled,
    RequiresApproval,
}

/// Returns whether the running executable is packaged as a `.app` bundle and can be used as a login item.
pub fn current_app_supports_login_item() -> bool {
    current_login_item_target().is_some()
}

/// Returns whether the current app bundle is configured to launch at login.
pub fn current_app_login_item_enabled() -> Result<Option<bool>> {
    let Some(target) = current_login_item_target() else {
        return Ok(None);
    };
    Ok(Some(login_item_enabled(&target)?))
}

/// Toggles whether the current app bundle is opened at login and returns the new enabled state.
pub fn toggle_current_app_login_item() -> Result<Option<bool>> {
    let Some(target) = current_login_item_target() else {
        return Ok(None);
    };

    if sm_app_service_supported() {
        toggle_sm_login_item(&target).map(Some)
    } else {
        toggle_launch_agent_login_item(&target).map(Some)
    }
}

fn current_login_item_target() -> Option<LoginItemTarget> {
    let executable = std::env::current_exe().ok()?;
    login_item_target_from_executable_path(executable.as_path())
}

pub(super) fn login_item_target_from_executable_path(
    executable_path: &Path,
) -> Option<LoginItemTarget> {
    bundle_root_from_executable_path(&executable_path.to_string_lossy())?;
    Some(LoginItemTarget {
        executable_path: executable_path.to_string_lossy().into_owned(),
    })
}

fn login_item_enabled(target: &LoginItemTarget) -> Result<bool> {
    if sm_app_service_supported() {
        let sm_enabled = sm_login_item_status()? == SmLoginItemStatus::Enabled;
        return Ok(sm_enabled || launch_agent_login_item_enabled(target)?);
    }

    launch_agent_login_item_enabled(target)
}

fn toggle_sm_login_item(target: &LoginItemTarget) -> Result<bool> {
    let sm_status = sm_login_item_status()?;
    if sm_status == SmLoginItemStatus::Enabled || launch_agent_login_item_enabled(target)? {
        unregister_sm_login_item()?;
        remove_login_agent()?;
        return Ok(false);
    }

    if sm_status == SmLoginItemStatus::RequiresApproval {
        open_sm_login_item_settings();
        bail!("Launch at Login needs approval in System Settings > General > Login Items");
    }

    register_sm_login_item()?;
    remove_login_agent()?;
    Ok(true)
}

fn toggle_launch_agent_login_item(target: &LoginItemTarget) -> Result<bool> {
    let enabled = launch_agent_login_item_enabled(target)?;
    if enabled {
        remove_login_agent()?;
        Ok(false)
    } else {
        write_login_agent(target)?;
        Ok(true)
    }
}

fn launch_agent_login_item_enabled(target: &LoginItemTarget) -> Result<bool> {
    let Some(path) = login_agent_plist_path() else {
        return Ok(false);
    };
    if !path.exists() {
        return Ok(false);
    }

    let plist =
        Value::from_file(&path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(login_agent_plist_matches_target(&plist, target))
}

fn sm_app_service_supported() -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 13,
        minorVersion: 0,
        patchVersion: 0,
    })
}

fn sm_login_item_status() -> Result<SmLoginItemStatus> {
    let status = unsafe { SMAppService::mainAppService().status() };
    match status {
        SMAppServiceStatus::Enabled => Ok(SmLoginItemStatus::Enabled),
        SMAppServiceStatus::NotRegistered | SMAppServiceStatus::NotFound => {
            Ok(SmLoginItemStatus::Disabled)
        }
        SMAppServiceStatus::RequiresApproval => Ok(SmLoginItemStatus::RequiresApproval),
        other => bail!("unexpected SMAppService status {:?}", other),
    }
}

fn register_sm_login_item() -> Result<()> {
    let service = unsafe { SMAppService::mainAppService() };
    match unsafe { service.registerAndReturnError() } {
        Ok(()) => Ok(()),
        Err(error) if error.code() == kSMErrorAlreadyRegistered as isize => Ok(()),
        Err(error) => Err(sm_app_service_error("register", &error)),
    }
}

fn unregister_sm_login_item() -> Result<()> {
    let service = unsafe { SMAppService::mainAppService() };
    match unsafe { service.unregisterAndReturnError() } {
        Ok(()) => Ok(()),
        Err(error) if error.code() == kSMErrorJobNotFound as isize => Ok(()),
        Err(error) => Err(sm_app_service_error("unregister", &error)),
    }
}

fn open_sm_login_item_settings() {
    unsafe { SMAppService::openSystemSettingsLoginItems() };
}

fn sm_app_service_error(action: &str, error: &NSError) -> anyhow::Error {
    anyhow::anyhow!(
        "failed to {action} Launch at Login through SMAppService: {}",
        error.localizedDescription()
    )
}

fn write_login_agent(target: &LoginItemTarget) -> Result<()> {
    let path = login_agent_plist_path().context("failed to resolve ~/Library/LaunchAgents")?;
    let parent = path
        .parent()
        .context("failed to resolve LaunchAgents directory")?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let plist = login_agent_plist_value(target);
    plist
        .to_file_xml(&path)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn remove_login_agent() -> Result<()> {
    let Some(path) = login_agent_plist_path() else {
        return Ok(());
    };
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("failed to remove {}", path.display()));
        }
    }
    Ok(())
}

fn login_agent_plist_path() -> Option<PathBuf> {
    Some(
        PathBuf::from(std::env::var_os("HOME")?)
            .join("Library")
            .join("LaunchAgents")
            .join(LOGIN_AGENT_PLIST_NAME),
    )
}

pub(super) fn login_agent_plist_value(target: &LoginItemTarget) -> Value {
    let mut dict = Dictionary::new();
    dict.insert(
        "Label".to_owned(),
        Value::String(LOGIN_AGENT_LABEL.to_owned()),
    );
    dict.insert(
        "ProgramArguments".to_owned(),
        Value::Array(vec![Value::String(target.executable_path.clone())]),
    );
    dict.insert("RunAtLoad".to_owned(), Value::Boolean(true));
    Value::Dictionary(dict)
}

pub(super) fn login_agent_plist_matches_target(plist: &Value, target: &LoginItemTarget) -> bool {
    let Some(dict) = plist.as_dictionary() else {
        return false;
    };
    let Some(label) = dict.get("Label").and_then(Value::as_string) else {
        return false;
    };
    let Some(arguments) = dict.get("ProgramArguments").and_then(Value::as_array) else {
        return false;
    };
    let Some(program) = arguments.first().and_then(Value::as_string) else {
        return false;
    };
    let run_at_load = dict
        .get("RunAtLoad")
        .and_then(Value::as_boolean)
        .unwrap_or(false);

    label == LOGIN_AGENT_LABEL && program == target.executable_path && run_at_load
}
