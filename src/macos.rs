//! macOS-specific shell-outs and permission-sensitive integrations.
//!
//! If something talks to `open`, AppKit, Accessibility, or Quartz events
//! trust, it belongs here rather than leaking platform details into the rest of
//! the launcher.

use std::{
    ffi::{c_char, c_float, c_int, c_void},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use core_foundation::{
    array::CFArray, base::TCFType, boolean::CFBoolean, data::CFData, dictionary::CFDictionary,
    string::CFString,
};
use core_foundation_sys::{
    base::{Boolean, CFRelease, CFTypeRef},
    data::CFDataRef,
    dictionary::CFDictionaryRef,
    string::CFStringRef,
};
use core_graphics::{
    display::CGDisplay,
    event::{CGEvent, CGEventFlags, CGEventTapLocation, KeyCode},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use objc2_app_kit::{
    NSApplicationActivationOptions, NSApplicationActivationPolicy, NSPasteboard,
    NSPasteboardTypeString, NSRunningApplication, NSWindow, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSError, NSOperatingSystemVersion, NSProcessInfo, NSString};
use objc2_service_management::{
    SMAppService, SMAppServiceStatus, kSMErrorAlreadyRegistered, kSMErrorJobNotFound,
};
use plist::{Dictionary, Value};
use tao::{platform::macos::WindowExtMacOS, window::Window};

use crate::debug_log;

const PRIVACY_ACCESSIBILITY: &str = "Privacy_Accessibility";
const PRIVACY_SCREEN_CAPTURE: &str = "Privacy_ScreenCapture";
const APP_REACTIVATION_DELAY: Duration = Duration::from_millis(120);
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(250);
const AX_MESSAGING_TIMEOUT_SECONDS: c_float = 1.0;
const REMOTE_AX_WINDOW_SCAN_BUDGET: Duration = Duration::from_millis(100);
const REMOTE_AX_WINDOW_SCAN_LIMIT: u64 = 1_000;
const REMOTE_AX_TOKEN_MAGIC: i32 = 0x636f636f;
const RTLD_LAZY: c_int = 0x1;
const RTLD_LOCAL: c_int = 0x4;
const CPS_USER_GENERATED: u32 = 0x200;
const MAKE_KEY_EVENT_BYTES: usize = 0xf8;
const LOGIN_AGENT_LABEL: &str = "io.github.sloppish.runx.login-item";
const LOGIN_AGENT_PLIST_NAME: &str = "io.github.sloppish.runx.login-item.plist";

type AXUIElementRef = *const c_void;
type AXError = c_int;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct ProcessSerialNumber {
    high_long_of_psn: u32,
    low_long_of_psn: u32,
}

const K_AX_ERROR_SUCCESS: AXError = 0;
const K_AX_ERROR_CANNOT_COMPLETE: AXError = -25204;
const K_AX_ERROR_ATTRIBUTE_UNSUPPORTED: AXError = -25205;
const K_AX_ERROR_ACTION_UNSUPPORTED: AXError = -25206;
const K_AX_ERROR_API_DISABLED: AXError = -25211;
const K_AX_ERROR_NO_VALUE: AXError = -25212;

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;

    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> Boolean;
    fn AXUIElementCreateApplication(pid: c_int) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetMessagingTimeout(
        element: AXUIElementRef,
        timeout_in_seconds: c_float,
    ) -> AXError;
    fn GetProcessForPID(pid: c_int, psn: *mut ProcessSerialNumber) -> c_int;
    fn _AXUIElementGetWindow(element: AXUIElementRef, out: *mut u32) -> AXError;
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// Snapshot of the app that was frontmost before Runx appeared.
#[derive(Debug, Clone, Default)]
pub struct FrontmostApp {
    pub name: Option<String>,
    pub bundle_id: Option<String>,
    pub path: Option<String>,
}

/// Direct CoreGraphics snapshot of the display currently containing the mouse cursor.
#[derive(Debug, Clone, Copy)]
pub struct CursorDisplayLocation {
    pub display_id: u32,
}

/// Window metadata exposed through macOS Accessibility.
#[derive(Debug, Clone)]
pub struct AccessibilityWindow {
    pub window_id: u32,
    pub title: String,
    pub subrole: String,
}

#[derive(Debug, Clone)]
struct LoginItemTarget {
    executable_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmLoginItemStatus {
    Enabled,
    Disabled,
    RequiresApproval,
}

#[derive(Clone, Copy)]
enum AppActivationMode {
    Default,
    AllWindows,
}

#[derive(Debug, Clone, Copy)]
struct ExactWindowFocus {
    activated_app: bool,
}

struct WindowServerApis {
    set_front_process_with_options:
        unsafe extern "C" fn(*mut ProcessSerialNumber, u32, u32) -> c_int,
    post_event_record_to: unsafe extern "C" fn(*mut ProcessSerialNumber, *mut u8) -> c_int,
}

struct CoreGraphicsApis {
    preflight_screen_capture_access: unsafe extern "C" fn() -> bool,
    request_screen_capture_access: unsafe extern "C" fn() -> bool,
}

struct ApplicationServicesApis {
    create_ax_element_with_remote_token: unsafe extern "C" fn(CFDataRef) -> AXUIElementRef,
}

/// Opens an application bundle path through Launch Services.
pub fn open_application(path: &str) -> Result<()> {
    run_quiet("open", &[path])
}

/// Opens a System Settings deep-link URL.
pub fn open_settings(url: &str) -> Result<()> {
    run_quiet("open", &[url])
}

/// Writes plain text to the macOS clipboard using the native pasteboard API.
pub fn copy_text_to_clipboard(text: &str) -> Result<String> {
    write_clipboard_text(text)?;
    Ok("Copied to clipboard".to_owned())
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

/// Returns the display currently containing the mouse cursor using CoreGraphics.
pub fn cursor_display_location() -> Option<CursorDisplayLocation> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState).ok()?;
    let event = CGEvent::new(source).ok()?;
    let point = event.location();
    let (display_ids, matching_count) = CGDisplay::displays_with_point(point, 8).ok()?;
    if matching_count == 0 {
        return None;
    }

    let display_id = display_ids
        .into_iter()
        .take(matching_count as usize)
        .next()?;

    Some(CursorDisplayLocation { display_id })
}

/// Configures the launcher window to behave like a non-activating panel.
pub fn configure_launcher_panel(window: &Window) {
    let window = ns_window(window);
    let mut style_mask = window.styleMask();
    style_mask.insert(NSWindowStyleMask::NonactivatingPanel);
    window.setStyleMask(style_mask);
    window.setHidesOnDeactivate(false);
}

/// Shows the launcher panel without activating Runx as the foreground app.
pub fn show_launcher_panel(window: &Window) {
    let window = ns_window(window);
    window.orderFrontRegardless();
    window.makeKeyWindow();
}

/// Re-focuses the launcher panel without activating Runx as the foreground app.
pub fn focus_launcher_panel(window: &Window) {
    let window = ns_window(window);
    window.makeKeyWindow();
}

/// Focuses the selected window when possible, then activates the owning app.
/// Falls back to activating/opening the app if exact window focus is unavailable.
pub fn focus_window(app_name: &str, window_title: &str, window_id: u32) -> Result<Option<String>> {
    if let Some(pid) = running_application_pid_by_name(app_name) {
        if ensure_accessibility_trusted(true) {
            match focus_window_for_pid(pid, window_id) {
                Ok(focus) => {
                    if !focus.activated_app {
                        activate_running_application_by_pid_with_mode(
                            pid,
                            AppActivationMode::Default,
                        )?;
                        let _ = focus_window_for_pid(pid, window_id);
                    }
                    return Ok(Some(format!("Focused {}", window_title)));
                }
                Err(error) => {
                    debug_log::append(format!(
                        "focus_window direct focus failed app={:?} title={:?} window_id={} error={error:#}",
                        app_name, window_title, window_id
                    ));
                }
            }
        } else {
            open_accessibility_settings();
        }

        activate_running_application_by_pid(pid)?;
        return Ok(Some(format!("Activated {}", app_name)));
    }

    open_named_application(app_name)
        .with_context(|| format!("failed to activate {app_name} for window focus"))?;
    Ok(Some(format!("Activated {}", app_name)))
}

pub fn focus_window_and_activate_all_windows(
    app_name: &str,
    window_title: &str,
    window_id: u32,
) -> Result<Option<String>> {
    if let Some(pid) = running_application_pid_by_name(app_name) {
        let accessibility_trusted = ensure_accessibility_trusted(true);
        if accessibility_trusted {
            if let Err(error) = focus_window_for_pid(pid, window_id) {
                debug_log::append(format!(
                    "focus_window_and_activate_all_windows initial focus failed app={:?} title={:?} window_id={} error={error:#}",
                    app_name, window_title, window_id
                ));
            }
        } else {
            open_accessibility_settings();
        }

        activate_running_application_by_pid_with_mode(pid, AppActivationMode::AllWindows)?;

        if accessibility_trusted {
            match focus_window_for_pid(pid, window_id) {
                Ok(_) => {
                    return Ok(Some(format!(
                        "Focused {} and activated all {} windows",
                        window_title, app_name
                    )));
                }
                Err(error) => {
                    debug_log::append(format!(
                        "focus_window_and_activate_all_windows final focus failed app={:?} title={:?} window_id={} error={error:#}",
                        app_name, window_title, window_id
                    ));
                }
            }
        } else {
            return Ok(Some(format!(
                "Activated all {} windows (window focus requires Accessibility permission)",
                app_name
            )));
        }

        return Ok(Some(format!("Activated all {} windows", app_name)));
    }

    open_named_application(app_name)
        .with_context(|| format!("failed to activate {app_name} for all-window focus"))?;
    Ok(Some(format!("Activated {}", app_name)))
}

/// Types text into the app that was focused before Runx was shown.
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

    if requires_clipboard_paste(text) {
        debug_log::append("type_text_into_previous_app using clipboard paste path");
        return paste_text_into_previous_app(text);
    }

    post_keyboard_text(text).context("failed to post keyboard events for text typing")?;
    Ok("Typed into the previous app".to_owned())
}

fn paste_text_into_previous_app(text: &str) -> Result<String> {
    let previous_clipboard = read_clipboard_text().ok();
    write_clipboard_text(text)?;
    post_command_v().context("failed to post keyboard events for clipboard paste")?;
    schedule_clipboard_restore(previous_clipboard, text.to_owned());
    Ok("Typed into the previous app".to_owned())
}

fn schedule_clipboard_restore(previous_clipboard: Option<String>, inserted_text: String) {
    let Some(previous_clipboard) = previous_clipboard else {
        return;
    };

    thread::spawn(move || {
        thread::sleep(CLIPBOARD_RESTORE_DELAY);

        match read_clipboard_text() {
            Ok(current) if current == inserted_text => {
                if let Err(error) = write_clipboard_text(&previous_clipboard) {
                    debug_log::append(format!("clipboard restore failed after paste: {error:#}"));
                }
            }
            Ok(_) => {}
            Err(error) => debug_log::append(format!(
                "clipboard restore skipped after paste; could not read clipboard: {error:#}"
            )),
        }
    });
}

fn requires_clipboard_paste(text: &str) -> bool {
    !text.is_ascii()
}

/// Returns whether the current process is trusted for Accessibility APIs.
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

/// Returns whether the current process is trusted for Screen Recording APIs.
pub fn ensure_screen_recording_trusted(prompt: bool) -> bool {
    if !screen_recording_permission_supported() {
        return true;
    }

    let Some(apis) = core_graphics_apis() else {
        return false;
    };

    if unsafe { (apis.preflight_screen_capture_access)() } {
        return true;
    }

    if !prompt {
        return false;
    }

    let trusted = unsafe { (apis.request_screen_capture_access)() };
    if !trusted {
        open_screen_recording_settings();
    }
    trusted
}

/// Returns Accessibility-visible windows for a running application.
pub fn accessibility_windows_for_pid(pid: i64) -> Vec<AccessibilityWindow> {
    let Some(app_element) = OwnedAxElement::application(pid as c_int) else {
        return Vec::new();
    };
    let _ = app_element.set_messaging_timeout(AX_MESSAGING_TIMEOUT_SECONDS);

    let Ok(Some(windows)) =
        app_element.copy_array_attribute(ax_windows_attribute().as_concrete_TypeRef())
    else {
        return Vec::new();
    };

    let mut output = Vec::new();
    for raw_window in windows.get_all_values() {
        let window = raw_window.cast();
        let Some(window_id) = copy_ax_window_id(window).ok().flatten() else {
            continue;
        };
        let title = copy_ax_string(window, ax_title_attribute().as_concrete_TypeRef())
            .unwrap_or_default()
            .unwrap_or_default();
        let subrole = copy_ax_string(window, ax_subrole_attribute().as_concrete_TypeRef())
            .unwrap_or_default()
            .unwrap_or_default();
        output.push(AccessibilityWindow {
            window_id,
            title,
            subrole,
        });
    }
    output
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

fn open_accessibility_settings() {
    open_privacy_settings(PRIVACY_ACCESSIBILITY);
}

fn open_screen_recording_settings() {
    open_privacy_settings(PRIVACY_SCREEN_CAPTURE);
}

fn current_login_item_target() -> Option<LoginItemTarget> {
    let executable = std::env::current_exe().ok()?;
    login_item_target_from_executable_path(executable.as_path())
}

fn login_item_target_from_executable_path(executable_path: &Path) -> Option<LoginItemTarget> {
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

fn screen_recording_permission_supported() -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 10,
        minorVersion: 15,
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

fn login_agent_plist_value(target: &LoginItemTarget) -> Value {
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

fn login_agent_plist_matches_target(plist: &Value, target: &LoginItemTarget) -> bool {
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

fn open_privacy_settings(anchor: &str) {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
    let _ = run_quiet("open", &[url.as_str()]);
}

fn open_named_application(name: &str) -> Result<()> {
    run_quiet("open", &["-a", name])
}

fn running_application_pid_by_name(name: &str) -> Option<c_int> {
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

fn activate_running_application_by_pid(pid: c_int) -> Result<()> {
    activate_running_application_by_pid_with_mode(pid, AppActivationMode::Default)
}

fn activate_running_application_by_pid_with_mode(
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

fn focus_window_for_pid(pid: c_int, window_id: u32) -> Result<ExactWindowFocus> {
    let app_element = OwnedAxElement::application(pid)
        .ok_or_else(|| anyhow::anyhow!("failed to create accessibility handle for pid {pid}"))?;
    let _ = app_element.set_messaging_timeout(AX_MESSAGING_TIMEOUT_SECONDS);

    let windows = app_element
        .copy_array_attribute(ax_windows_attribute().as_concrete_TypeRef())
        .map_err(|error| anyhow::anyhow!(ax_error_message(error)))?;
    let Some(windows) = windows else {
        bail!("window list unavailable");
    };

    for raw_window in windows.get_all_values() {
        let window = raw_window.cast();
        let candidate_window_id = match copy_ax_window_id(window) {
            Ok(Some(window_id)) => window_id,
            Ok(None) => continue,
            Err(error) => {
                debug_log::append(format!(
                    "focus_window skipped inaccessible AX window pid={pid} error={error:#}"
                ));
                continue;
            }
        };
        if candidate_window_id != window_id {
            continue;
        }

        return focus_ax_window(pid, window_id, window);
    }

    if let Some(window) = remote_ax_window_for_pid(pid, window_id) {
        return focus_ax_window(pid, window_id, window.as_ptr());
    }

    bail!("window not found")
}

fn focus_ax_window(pid: c_int, window_id: u32, window: AXUIElementRef) -> Result<ExactWindowFocus> {
    let activated_app = match focus_cg_window(pid, window_id) {
        Ok(()) => true,
        Err(error) => {
            debug_log::append(format!(
                "focus_cg_window failed pid={pid} window_id={window_id} error={error:#}"
            ));
            false
        }
    };

    let mut did_focus = activated_app;
    if set_ax_bool_attribute(window, ax_main_attribute().as_concrete_TypeRef(), true).is_ok() {
        did_focus = true;
    }
    if set_ax_bool_attribute(window, ax_focused_attribute().as_concrete_TypeRef(), true).is_ok() {
        did_focus = true;
    }
    if perform_ax_action(window, ax_raise_action().as_concrete_TypeRef()).is_ok() {
        did_focus = true;
    }

    if did_focus {
        return Ok(ExactWindowFocus { activated_app });
    }

    bail!("window exists but does not expose focus actions")
}

fn remote_ax_window_for_pid(pid: c_int, window_id: u32) -> Option<OwnedAxElement> {
    let apis = application_services_apis()?;
    let mut token = [0_u8; 20];
    token[0..4].copy_from_slice(&pid.to_ne_bytes());
    token[8..12].copy_from_slice(&REMOTE_AX_TOKEN_MAGIC.to_ne_bytes());

    let started_at = Instant::now();
    for ax_id in 0..REMOTE_AX_WINDOW_SCAN_LIMIT {
        token[12..20].copy_from_slice(&ax_id.to_ne_bytes());
        let token_data = CFData::from_buffer(&token);
        let value =
            unsafe { (apis.create_ax_element_with_remote_token)(token_data.as_concrete_TypeRef()) };
        if value.is_null() {
            continue;
        }

        let element = OwnedAxElement(value);
        if matches!(copy_ax_window_id(element.as_ptr()).ok().flatten(), Some(id) if id == window_id)
        {
            return Some(element);
        }

        if started_at.elapsed() >= REMOTE_AX_WINDOW_SCAN_BUDGET {
            return None;
        }
    }

    None
}

fn focus_cg_window(pid: c_int, window_id: u32) -> Result<()> {
    let apis = window_server_apis().ok_or_else(|| {
        anyhow::anyhow!("SkyLight window focus APIs are unavailable on this macOS installation")
    })?;
    let mut psn = ProcessSerialNumber::default();
    let status = unsafe { GetProcessForPID(pid, &mut psn) };
    if status != 0 {
        bail!("failed to resolve process serial number for pid {pid}: status {status}");
    }

    let error =
        unsafe { (apis.set_front_process_with_options)(&mut psn, window_id, CPS_USER_GENERATED) };
    if error != 0 {
        bail!("failed to set front process for window {window_id}: error {error}");
    }

    make_key_window(apis, &mut psn, window_id)
}

fn make_key_window(
    apis: &WindowServerApis,
    psn: &mut ProcessSerialNumber,
    window_id: u32,
) -> Result<()> {
    let mut bytes = [0_u8; MAKE_KEY_EVENT_BYTES];
    bytes[0x04] = MAKE_KEY_EVENT_BYTES as u8;
    bytes[0x3a] = 0x10;
    bytes[0x3c..0x40].copy_from_slice(&window_id.to_ne_bytes());
    bytes[0x20..0x30].fill(0xff);

    bytes[0x08] = 0x01;
    let first_error = unsafe { (apis.post_event_record_to)(psn, bytes.as_mut_ptr()) };
    if first_error != 0 {
        bail!("failed to post key-window begin event: error {first_error}");
    }

    bytes[0x08] = 0x02;
    let second_error = unsafe { (apis.post_event_record_to)(psn, bytes.as_mut_ptr()) };
    if second_error != 0 {
        bail!("failed to post key-window end event: error {second_error}");
    }

    Ok(())
}

fn window_server_apis() -> Option<&'static WindowServerApis> {
    static APIS: OnceLock<Option<WindowServerApis>> = OnceLock::new();
    APIS.get_or_init(load_window_server_apis).as_ref()
}

fn core_graphics_apis() -> Option<&'static CoreGraphicsApis> {
    static APIS: OnceLock<Option<CoreGraphicsApis>> = OnceLock::new();
    APIS.get_or_init(load_core_graphics_apis).as_ref()
}

fn application_services_apis() -> Option<&'static ApplicationServicesApis> {
    static APIS: OnceLock<Option<ApplicationServicesApis>> = OnceLock::new();
    APIS.get_or_init(load_application_services_apis).as_ref()
}

fn load_core_graphics_apis() -> Option<CoreGraphicsApis> {
    let handle = unsafe {
        dlopen(
            c"/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics".as_ptr(),
            RTLD_LAZY | RTLD_LOCAL,
        )
    };
    if handle.is_null() {
        return None;
    }

    Some(CoreGraphicsApis {
        preflight_screen_capture_access: load_symbol(handle, "CGPreflightScreenCaptureAccess")?,
        request_screen_capture_access: load_symbol(handle, "CGRequestScreenCaptureAccess")?,
    })
}

fn load_application_services_apis() -> Option<ApplicationServicesApis> {
    let handle = unsafe {
        dlopen(
            c"/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices"
                .as_ptr(),
            RTLD_LAZY | RTLD_LOCAL,
        )
    };
    if handle.is_null() {
        return None;
    }

    Some(ApplicationServicesApis {
        create_ax_element_with_remote_token: load_symbol(
            handle,
            "_AXUIElementCreateWithRemoteToken",
        )?,
    })
}

fn load_window_server_apis() -> Option<WindowServerApis> {
    let handle = unsafe {
        dlopen(
            c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight".as_ptr(),
            RTLD_LAZY | RTLD_LOCAL,
        )
    };
    if handle.is_null() {
        return None;
    }

    Some(WindowServerApis {
        set_front_process_with_options: load_symbol(handle, "_SLPSSetFrontProcessWithOptions")?,
        post_event_record_to: load_symbol(handle, "SLPSPostEventRecordTo")?,
    })
}

fn load_symbol<T>(handle: *mut c_void, name: &str) -> Option<T> {
    let mut symbol_name = Vec::with_capacity(name.len() + 1);
    symbol_name.extend_from_slice(name.as_bytes());
    symbol_name.push(0);

    let symbol = unsafe { dlsym(handle, symbol_name.as_ptr().cast()) };
    if symbol.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute_copy(&symbol) })
}

fn ns_window(window: &Window) -> &NSWindow {
    unsafe { &*(window.ns_window() as *mut NSWindow) }
}

fn bundle_root_from_executable_path(executable_path: &str) -> Option<String> {
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

fn ax_focused_attribute() -> CFString {
    CFString::from_static_string("AXFocused")
}

fn ax_main_attribute() -> CFString {
    CFString::from_static_string("AXMain")
}

fn ax_raise_action() -> CFString {
    CFString::from_static_string("AXRaise")
}

fn ax_title_attribute() -> CFString {
    CFString::from_static_string("AXTitle")
}

fn ax_subrole_attribute() -> CFString {
    CFString::from_static_string("AXSubrole")
}

fn ax_windows_attribute() -> CFString {
    CFString::from_static_string("AXWindows")
}

fn read_clipboard_text() -> Result<String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let text = pasteboard
        .stringForType(pasteboard_type_string())
        .ok_or_else(|| anyhow::anyhow!("clipboard does not currently contain plain text"))?;
    Ok(text.to_string())
}

fn write_clipboard_text(text: &str) -> Result<()> {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    let text = NSString::from_str(text);
    if pasteboard.setString_forType(&text, pasteboard_type_string()) {
        return Ok(());
    }
    bail!("failed to write plain text to the macOS pasteboard")
}

fn pasteboard_type_string() -> &'static objc2_app_kit::NSPasteboardType {
    // SAFETY: Apple exports `NSPasteboardTypeString` as a process-global constant.
    unsafe { NSPasteboardTypeString }
}

fn post_keyboard_text(text: &str) -> Result<()> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| anyhow::anyhow!("failed to create Quartz event source"))?;
    let key_down = CGEvent::new_keyboard_event(source.clone(), 0, true)
        .map_err(|_| anyhow::anyhow!("failed to create key-down event"))?;
    key_down.set_string(text);
    key_down.post(CGEventTapLocation::HID);

    let key_up = CGEvent::new_keyboard_event(source, 0, false)
        .map_err(|_| anyhow::anyhow!("failed to create key-up event"))?;
    key_up.set_string(text);
    key_up.post(CGEventTapLocation::HID);
    Ok(())
}

fn post_command_v() -> Result<()> {
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| anyhow::anyhow!("failed to create Quartz event source"))?;

    let command_down = CGEvent::new_keyboard_event(source.clone(), KeyCode::COMMAND, true)
        .map_err(|_| anyhow::anyhow!("failed to create command key-down event"))?;
    command_down.set_flags(CGEventFlags::CGEventFlagCommand);
    command_down.post(CGEventTapLocation::HID);

    let v_down = CGEvent::new_keyboard_event(source.clone(), KeyCode::ANSI_V, true)
        .map_err(|_| anyhow::anyhow!("failed to create V key-down event"))?;
    v_down.set_flags(CGEventFlags::CGEventFlagCommand);
    v_down.post(CGEventTapLocation::HID);

    let v_up = CGEvent::new_keyboard_event(source.clone(), KeyCode::ANSI_V, false)
        .map_err(|_| anyhow::anyhow!("failed to create V key-up event"))?;
    v_up.set_flags(CGEventFlags::CGEventFlagCommand);
    v_up.post(CGEventTapLocation::HID);

    let command_up = CGEvent::new_keyboard_event(source, KeyCode::COMMAND, false)
        .map_err(|_| anyhow::anyhow!("failed to create command key-up event"))?;
    command_up.set_flags(CGEventFlags::CGEventFlagNull);
    command_up.post(CGEventTapLocation::HID);
    Ok(())
}

fn copy_ax_window_id(element: AXUIElementRef) -> Result<Option<u32>> {
    let mut value = 0_u32;
    let error = unsafe { _AXUIElementGetWindow(element, &mut value) };
    match error {
        K_AX_ERROR_SUCCESS => Ok(Some(value)),
        K_AX_ERROR_ATTRIBUTE_UNSUPPORTED | K_AX_ERROR_NO_VALUE => Ok(None),
        other => bail!(ax_error_message(other)),
    }
}

fn copy_ax_string(element: AXUIElementRef, attribute: CFStringRef) -> Result<Option<String>> {
    let Some(value) = copy_ax_attribute_value(element, attribute)
        .map_err(|error| anyhow::anyhow!(ax_error_message(error)))?
    else {
        return Ok(None);
    };
    Ok(value.downcast::<CFString>().map(|value| value.to_string()))
}

fn copy_ax_attribute_value(
    element: AXUIElementRef,
    attribute: CFStringRef,
) -> std::result::Result<Option<core_foundation::base::CFType>, AXError> {
    let mut value: CFTypeRef = std::ptr::null();
    let error = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    match error {
        K_AX_ERROR_SUCCESS if value.is_null() => Ok(None),
        K_AX_ERROR_SUCCESS => Ok(Some(unsafe {
            core_foundation::base::CFType::wrap_under_create_rule(value)
        })),
        K_AX_ERROR_NO_VALUE => Ok(None),
        other => Err(other),
    }
}

fn set_ax_bool_attribute(
    element: AXUIElementRef,
    attribute: CFStringRef,
    value: bool,
) -> std::result::Result<(), AXError> {
    let boolean = if value {
        CFBoolean::true_value()
    } else {
        CFBoolean::false_value()
    };
    let error = unsafe { AXUIElementSetAttributeValue(element, attribute, boolean.as_CFTypeRef()) };
    match error {
        K_AX_ERROR_SUCCESS => Ok(()),
        other => Err(other),
    }
}

fn perform_ax_action(
    element: AXUIElementRef,
    action: CFStringRef,
) -> std::result::Result<(), AXError> {
    let error = unsafe { AXUIElementPerformAction(element, action) };
    match error {
        K_AX_ERROR_SUCCESS => Ok(()),
        other => Err(other),
    }
}

fn ax_error_message(error: AXError) -> &'static str {
    match error {
        K_AX_ERROR_SUCCESS => "success",
        K_AX_ERROR_CANNOT_COMPLETE => "the target app did not respond to Accessibility",
        K_AX_ERROR_ATTRIBUTE_UNSUPPORTED => {
            "the target window does not expose the requested Accessibility attribute"
        }
        K_AX_ERROR_ACTION_UNSUPPORTED => {
            "the target window does not expose the requested Accessibility action"
        }
        K_AX_ERROR_API_DISABLED => "Accessibility access is disabled for this process",
        K_AX_ERROR_NO_VALUE => "the requested Accessibility value is missing",
        _ => "an unknown Accessibility error occurred",
    }
}

struct OwnedAxElement(AXUIElementRef);

impl OwnedAxElement {
    fn application(pid: c_int) -> Option<Self> {
        let value = unsafe { AXUIElementCreateApplication(pid) };
        if value.is_null() {
            return None;
        }
        Some(Self(value))
    }

    fn as_ptr(&self) -> AXUIElementRef {
        self.0
    }

    fn set_messaging_timeout(
        &self,
        timeout_in_seconds: c_float,
    ) -> std::result::Result<(), AXError> {
        let error = unsafe { AXUIElementSetMessagingTimeout(self.as_ptr(), timeout_in_seconds) };
        match error {
            K_AX_ERROR_SUCCESS => Ok(()),
            other => Err(other),
        }
    }

    fn copy_array_attribute(
        &self,
        attribute: CFStringRef,
    ) -> std::result::Result<Option<CFArray>, AXError> {
        let Some(value) = copy_ax_attribute_value(self.as_ptr(), attribute)? else {
            return Ok(None);
        };
        Ok(value.downcast_into::<CFArray>())
    }
}

impl Drop for OwnedAxElement {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.0 as _);
        }
    }
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
    use std::path::{Path, PathBuf};

    use super::{
        LOGIN_AGENT_LABEL, bundle_root_from_executable_path, login_agent_plist_matches_target,
        login_agent_plist_value, login_item_target_from_executable_path, requires_clipboard_paste,
    };

    #[test]
    fn extracts_bundle_root_from_executable_path() {
        assert_eq!(
            bundle_root_from_executable_path(
                "/Applications/SampleApp.app/Contents/MacOS/sample-app"
            )
            .as_deref(),
            Some("/Applications/SampleApp.app")
        );
    }

    #[test]
    fn returns_none_when_executable_path_has_no_bundle_root() {
        assert_eq!(bundle_root_from_executable_path("/usr/bin/ssh"), None);
    }

    #[test]
    fn ascii_text_uses_keystroke_path() {
        assert!(!requires_clipboard_paste("hello123"));
    }

    #[test]
    fn emoji_uses_clipboard_paste_path() {
        assert!(requires_clipboard_paste("👍"));
    }

    #[test]
    fn derives_login_item_target_from_bundled_executable() {
        let target = login_item_target_from_executable_path(
            PathBuf::from("/Applications/Runx.app/Contents/MacOS/runx").as_path(),
        )
        .expect("bundled executable should produce login item target");

        assert_eq!(
            target.executable_path,
            "/Applications/Runx.app/Contents/MacOS/runx"
        );
    }

    #[test]
    fn login_item_target_is_unavailable_for_non_bundled_executable() {
        assert!(login_item_target_from_executable_path(Path::new("/usr/bin/ssh")).is_none());
    }

    #[test]
    fn login_agent_plist_targets_current_app_executable() {
        let target = login_item_target_from_executable_path(
            PathBuf::from("/Applications/Runx.app/Contents/MacOS/runx").as_path(),
        )
        .expect("bundled executable should produce login item target");

        let plist = login_agent_plist_value(&target);
        let dict = plist
            .as_dictionary()
            .expect("login agent plist should be a dictionary");

        assert_eq!(
            dict.get("Label").and_then(plist::Value::as_string),
            Some(LOGIN_AGENT_LABEL)
        );
        assert_eq!(
            dict.get("ProgramArguments")
                .and_then(plist::Value::as_array)
                .and_then(|arguments| arguments.first())
                .and_then(plist::Value::as_string),
            Some("/Applications/Runx.app/Contents/MacOS/runx")
        );
        assert_eq!(
            dict.get("RunAtLoad").and_then(plist::Value::as_boolean),
            Some(true)
        );
        assert!(login_agent_plist_matches_target(&plist, &target));
    }
}
