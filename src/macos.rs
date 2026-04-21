//! macOS-specific shell-outs and permission-sensitive integrations.
//!
//! If something talks to `open`, AppKit, Accessibility, or Quartz events
//! trust, it belongs here rather than leaking platform details into the rest of
//! the launcher.

use std::{
    ffi::{c_float, c_int, c_void},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use core_foundation::{
    array::CFArray, base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, string::CFString,
};
use core_foundation_sys::{
    base::{Boolean, CFRelease, CFTypeRef},
    dictionary::CFDictionaryRef,
    string::CFStringRef,
};
use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventTapLocation, KeyCode},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use objc2_app_kit::{
    NSApplicationActivationOptions, NSPasteboard, NSPasteboardTypeString, NSRunningApplication,
    NSWorkspace,
};
use objc2_foundation::NSString;

use crate::debug_log;

const PRIVACY_ACCESSIBILITY: &str = "Privacy_Accessibility";
const APP_REACTIVATION_DELAY: Duration = Duration::from_millis(120);
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(250);
const AX_MESSAGING_TIMEOUT_SECONDS: c_float = 1.0;

type AXUIElementRef = *const c_void;
type AXError = c_int;

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
    fn CGPreflightScreenCaptureAccess() -> Boolean;
    fn CGRequestScreenCaptureAccess() -> Boolean;
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
}

/// Snapshot of the app that was frontmost before Runx appeared.
#[derive(Debug, Clone, Default)]
pub struct FrontmostApp {
    pub name: Option<String>,
    pub bundle_id: Option<String>,
    pub path: Option<String>,
}

impl FrontmostApp {
    /// Human-friendly app name used in diagnostics and fallbacks.
    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .or(self.bundle_id.as_deref())
            .or(self.path.as_deref())
            .unwrap_or("previous app")
    }
}

/// Opens an application bundle path through Launch Services.
pub fn open_application(path: &str) -> Result<()> {
    run_quiet("open", &[path])
}

/// Opens an arbitrary path through Launch Services.
pub fn open_path(path: &str) -> Result<()> {
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

/// Attempts to focus a specific window, falling back to app activation.
pub fn focus_window(app_name: &str, window_title: &str) -> Result<Option<String>> {
    let Some(pid) = activate_running_application_by_name(app_name)? else {
        open_named_application(app_name)
            .with_context(|| format!("failed to activate {app_name} for window focus"))?;
        return Ok(Some(format!("Activated {}", app_name)));
    };

    if !ensure_accessibility_trusted(true) {
        open_accessibility_settings();
        return Ok(Some(format!(
            "Activated {} (window focus requires Accessibility permission)",
            app_name
        )));
    }

    match focus_window_for_pid(pid, window_title) {
        Ok(()) => Ok(Some(format!("Focused {}", window_title))),
        Err(error) => Ok(Some(format!(
            "Activated {} (direct window focus unavailable: {})",
            app_name, error
        ))),
    }
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

    reactivate_previous_app(previous_app)?;
    debug_log::append("type_text_into_previous_app reactivated previous app");

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

fn reactivate_previous_app(app: Option<&FrontmostApp>) -> Result<()> {
    let Some(app) = app else {
        debug_log::append("reactivate_previous_app skipped: no previous app");
        return Ok(());
    };

    if activate_matching_running_application(app)? {
        debug_log::append(format!(
            "reactivate_previous_app via running application name={:?} bundle_id={:?} path={:?}",
            app.name, app.bundle_id, app.path
        ));
        return Ok(());
    }

    if let Some(bundle_id) = app.bundle_id.as_deref()
        && run_quiet("open", &["-b", bundle_id]).is_ok()
    {
        debug_log::append(format!(
            "reactivate_previous_app fallback via bundle_id {:?}",
            bundle_id
        ));
        thread::sleep(APP_REACTIVATION_DELAY);
        return Ok(());
    }

    if let Some(path) = app.path.as_deref()
        && run_quiet("open", &["-a", path]).is_ok()
    {
        debug_log::append(format!(
            "reactivate_previous_app fallback via path {:?}",
            path
        ));
        thread::sleep(APP_REACTIVATION_DELAY);
        return Ok(());
    }

    if let Some(name) = app.name.as_deref()
        && run_quiet("open", &["-a", name]).is_ok()
    {
        debug_log::append(format!(
            "reactivate_previous_app fallback via name {:?}",
            name
        ));
        thread::sleep(APP_REACTIVATION_DELAY);
        return Ok(());
    }

    debug_log::append(format!(
        "reactivate_previous_app failed for name={:?} bundle_id={:?} path={:?}",
        app.name, app.bundle_id, app.path
    ));
    bail!("failed to reactivate {}", app.display_name());
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

/// Requests Screen Recording access at most once per process and returns whether it is granted.
pub fn request_screen_capture_access_once() -> bool {
    if unsafe { CGPreflightScreenCaptureAccess() != 0 } {
        return true;
    }

    let _ = unsafe { CGRequestScreenCaptureAccess() != 0 };

    unsafe { CGPreflightScreenCaptureAccess() != 0 }
}

fn open_accessibility_settings() {
    open_privacy_settings(PRIVACY_ACCESSIBILITY);
}

fn open_privacy_settings(anchor: &str) {
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
    let _ = run_quiet("open", &[url.as_str()]);
}

fn open_named_application(name: &str) -> Result<()> {
    run_quiet("open", &["-a", name])
}

fn activate_matching_running_application(app: &FrontmostApp) -> Result<bool> {
    if let Some(bundle_id) = app.bundle_id.as_deref() {
        let bundle_id = NSString::from_str(bundle_id);
        let matches = NSRunningApplication::runningApplicationsWithBundleIdentifier(&bundle_id);
        if matches.count() > 0 {
            activate_running_application(&matches.objectAtIndex(0))?;
            return Ok(true);
        }
    }

    let running_apps = NSWorkspace::sharedWorkspace().runningApplications();
    for index in 0..running_apps.count() {
        let candidate = running_apps.objectAtIndex(index);
        if app
            .path
            .as_deref()
            .zip(candidate.bundleURL())
            .and_then(|(expected, url)| url.path().map(|path| (expected, path.to_string())))
            .is_some_and(|(expected, actual)| expected == actual)
        {
            activate_running_application(&candidate)?;
            return Ok(true);
        }

        if app
            .name
            .as_deref()
            .zip(candidate.localizedName())
            .is_some_and(|(expected, actual)| expected == actual.to_string())
        {
            activate_running_application(&candidate)?;
            return Ok(true);
        }
    }

    Ok(false)
}

fn activate_running_application_by_name(name: &str) -> Result<Option<c_int>> {
    let running_apps = NSWorkspace::sharedWorkspace().runningApplications();
    for index in 0..running_apps.count() {
        let candidate = running_apps.objectAtIndex(index);
        if candidate
            .localizedName()
            .is_some_and(|actual| actual.to_string() == name)
        {
            let pid = candidate.processIdentifier();
            activate_running_application(&candidate)?;
            return Ok(Some(pid));
        }
    }
    Ok(None)
}

fn activate_running_application(app: &NSRunningApplication) -> Result<()> {
    if app.isHidden() {
        let _ = app.unhide();
    }

    if app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows) {
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

fn focus_window_for_pid(pid: c_int, window_title: &str) -> Result<()> {
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
        let title = copy_ax_string_attribute(window, ax_title_attribute().as_concrete_TypeRef())?;
        if title.as_deref() != Some(window_title) {
            continue;
        }

        let mut did_focus = false;
        if set_ax_bool_attribute(window, ax_main_attribute().as_concrete_TypeRef(), true).is_ok() {
            did_focus = true;
        }
        if set_ax_bool_attribute(window, ax_focused_attribute().as_concrete_TypeRef(), true).is_ok()
        {
            did_focus = true;
        }
        if perform_ax_action(window, ax_raise_action().as_concrete_TypeRef()).is_ok() {
            did_focus = true;
        }

        if did_focus {
            return Ok(());
        }

        bail!("window exists but does not expose focus actions");
    }

    bail!("window not found")
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

fn copy_ax_string_attribute(
    element: AXUIElementRef,
    attribute: CFStringRef,
) -> Result<Option<String>> {
    match copy_ax_attribute_value(element, attribute) {
        Ok(Some(value)) => Ok(value
            .downcast_into::<CFString>()
            .map(|text| text.to_string())),
        Ok(None) => Ok(None),
        Err(K_AX_ERROR_ATTRIBUTE_UNSUPPORTED | K_AX_ERROR_NO_VALUE) => Ok(None),
        Err(error) => bail!(ax_error_message(error)),
    }
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
    use std::path::Path;

    use super::{bundle_root_from_executable_path, requires_clipboard_paste};

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
    fn prefers_full_path_display_if_name_missing() {
        let app = super::FrontmostApp {
            name: None,
            bundle_id: None,
            path: Some("/Applications/SampleApp.app".to_owned()),
        };
        assert_eq!(app.display_name(), "/Applications/SampleApp.app");
        assert_eq!(
            Path::new(app.display_name())
                .file_name()
                .and_then(|value| value.to_str()),
            Some("SampleApp.app")
        );
    }

    #[test]
    fn ascii_text_uses_keystroke_path() {
        assert!(!requires_clipboard_paste("hello123"));
    }

    #[test]
    fn emoji_uses_clipboard_paste_path() {
        assert!(requires_clipboard_paste("👍"));
    }
}
