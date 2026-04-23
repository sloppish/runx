//! macOS-specific shell-outs and permission-sensitive integrations.
//!
//! If something talks to `open`, AppKit, Accessibility, or Quartz events
//! trust, it belongs here rather than leaking platform details into the rest of
//! the launcher.

use std::{
    ffi::{c_float, c_int, c_void},
    io::Write,
    path::Path,
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
    display::CGDisplay,
    event::{CGEvent, CGEventFlags, CGEventTapLocation, KeyCode},
    event_source::{CGEventSource, CGEventSourceStateID},
};
use objc2_app_kit::{
    NSApplicationActivationOptions, NSPasteboard, NSPasteboardTypeString, NSRunningApplication,
    NSWindow, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::NSString;
use tao::{platform::macos::WindowExtMacOS, window::Window};

use crate::{config::WindowFocusBehavior, debug_log};

const PRIVACY_ACCESSIBILITY: &str = "Privacy_Accessibility";
const APP_REACTIVATION_DELAY: Duration = Duration::from_millis(120);
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(250);
const AX_MESSAGING_TIMEOUT_SECONDS: c_float = 1.0;
const LOGIN_ITEM_SCRIPT: &str = r#"on run argv
  set actionName to item 1 of argv
  set appPath to item 2 of argv
  set appName to item 3 of argv

  tell application "System Events"
    if actionName is "enable" then
      set itemCount to count login items
      repeat with i from itemCount to 1 by -1
        set existingItem to login item i
        set itemPath to ""
        try
          set itemPath to path of existingItem as text
        end try
        if itemPath is appPath then
          delete existingItem
        end if
      end repeat
      make login item at end with properties {name:appName, path:appPath, hidden:false}
      return "enabled"
    else if actionName is "disable" then
      set removedAny to false
      set itemCount to count login items
      repeat with i from itemCount to 1 by -1
        set existingItem to login item i
        set itemPath to ""
        try
          set itemPath to path of existingItem as text
        end try
        if itemPath is appPath then
          delete existingItem
          set removedAny to true
        end if
      end repeat
      if removedAny then return "disabled"
      return "missing"
    else if actionName is "status" then
      set itemCount to count login items
      repeat with i from 1 to itemCount
        set existingItem to login item i
        set itemPath to ""
        try
          set itemPath to path of existingItem as text
        end try
        if itemPath is appPath then
          return "enabled"
        end if
      end repeat
      return "disabled"
    else
      error "unknown action"
    end if
  end tell
end run
"#;

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
    fn _AXUIElementGetWindow(element: AXUIElementRef, out: *mut u32) -> AXError;
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

#[derive(Debug, Clone)]
struct LoginItemTarget {
    path: String,
    name: String,
}

#[derive(Clone, Copy)]
enum AppActivationMode {
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

/// Attempts to focus a specific window using the configured strategy.
pub fn focus_window(
    app_name: &str,
    window_title: &str,
    window_id: u32,
    behavior: WindowFocusBehavior,
) -> Result<Option<String>> {
    match behavior {
        WindowFocusBehavior::ActivateAppOnly => activate_or_open_application(app_name),
        WindowFocusBehavior::ActivateAppThenFocusWindow => {
            activate_then_focus_window(app_name, window_title, window_id)
        }
        WindowFocusBehavior::FocusWindowOnly => {
            focus_window_only(app_name, window_title, window_id)
        }
        WindowFocusBehavior::FocusWindowThenActivateFallback => {
            focus_window_then_activate_fallback(app_name, window_title, window_id)
        }
    }
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
                Ok(()) => {
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

/// Requests Screen Recording access if needed and returns whether it is granted.
pub fn request_screen_capture_access_once() -> bool {
    if has_screen_capture_access() {
        return true;
    }

    let _ = unsafe { CGRequestScreenCaptureAccess() != 0 };

    has_screen_capture_access()
}

/// Returns whether macOS currently grants Screen Recording access to Runx.
pub fn has_screen_capture_access() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() != 0 }
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
    Ok(Some(login_item_status(&target)?))
}

/// Toggles whether the current app bundle is opened at login and returns the new enabled state.
pub fn toggle_current_app_login_item() -> Result<Option<bool>> {
    let Some(target) = current_login_item_target() else {
        return Ok(None);
    };

    let enabled = login_item_status(&target)?;
    let next_action = if enabled { "disable" } else { "enable" };
    let status = run_login_item_action(next_action, &target)?;
    Ok(Some(matches!(status.as_str(), "enabled")))
}

fn open_accessibility_settings() {
    open_privacy_settings(PRIVACY_ACCESSIBILITY);
}

fn current_login_item_target() -> Option<LoginItemTarget> {
    let executable = std::env::current_exe().ok()?;
    login_item_target_from_executable_path(executable.as_path())
}

fn login_item_target_from_executable_path(executable_path: &Path) -> Option<LoginItemTarget> {
    let bundle_path = bundle_root_from_executable_path(&executable_path.to_string_lossy())?;
    let name = Path::new(&bundle_path)
        .file_stem()
        .and_then(|value| value.to_str())?
        .to_owned();
    Some(LoginItemTarget {
        path: bundle_path,
        name,
    })
}

fn login_item_status(target: &LoginItemTarget) -> Result<bool> {
    let status = run_login_item_action("status", target)?;
    match status.as_str() {
        "enabled" => Ok(true),
        "disabled" | "missing" => Ok(false),
        other => bail!("unexpected login item status `{other}`"),
    }
}

fn run_login_item_action(action: &str, target: &LoginItemTarget) -> Result<String> {
    let mut child = Command::new("osascript")
        .arg("-")
        .arg(action)
        .arg(&target.path)
        .arg(&target.name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to launch `osascript` for login item management")?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to open osascript stdin"))?;
        stdin
            .write_all(LOGIN_ITEM_SCRIPT.as_bytes())
            .context("failed to send AppleScript to `osascript`")?;
    }

    let output = child
        .wait_with_output()
        .context("failed to wait for `osascript` login item command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.is_empty() {
            bail!("`osascript` exited with status {}", output.status);
        }
        bail!("login item command failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
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

fn activate_running_application_by_name(name: &str) -> Result<Option<c_int>> {
    let Some(pid) = running_application_pid_by_name(name) else {
        return Ok(None);
    };
    activate_running_application_by_pid(pid)?;
    Ok(Some(pid))
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

fn activate_or_open_application(app_name: &str) -> Result<Option<String>> {
    if activate_running_application_by_name(app_name)?.is_some() {
        return Ok(Some(format!("Activated {}", app_name)));
    }

    open_named_application(app_name)
        .with_context(|| format!("failed to activate {app_name} for window focus"))?;
    Ok(Some(format!("Activated {}", app_name)))
}

fn activate_then_focus_window(
    app_name: &str,
    window_title: &str,
    window_id: u32,
) -> Result<Option<String>> {
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

    match focus_window_for_pid(pid, window_id) {
        Ok(()) => Ok(Some(format!("Focused {}", window_title))),
        Err(error) => Ok(Some(format!(
            "Activated {} (direct window focus unavailable: {})",
            app_name, error
        ))),
    }
}

fn focus_window_only(app_name: &str, window_title: &str, window_id: u32) -> Result<Option<String>> {
    let Some(pid) = running_application_pid_by_name(app_name) else {
        bail!(
            "window is no longer available because {} is not running",
            app_name
        );
    };

    if !ensure_accessibility_trusted(true) {
        open_accessibility_settings();
        bail!(
            "Runx needs Accessibility permission to focus a specific window without activating the whole app."
        );
    }

    focus_window_for_pid(pid, window_id)?;
    Ok(Some(format!("Focused {}", window_title)))
}

fn focus_window_then_activate_fallback(
    app_name: &str,
    window_title: &str,
    window_id: u32,
) -> Result<Option<String>> {
    if let Some(pid) = running_application_pid_by_name(app_name) {
        if ensure_accessibility_trusted(true) {
            match focus_window_for_pid(pid, window_id) {
                Ok(()) => {
                    activate_running_application_by_pid_with_mode(pid, AppActivationMode::Default)?;
                    let _ = focus_window_for_pid(pid, window_id);
                    return Ok(Some(format!("Focused {}", window_title)));
                }
                Err(error) => {
                    debug_log::append(format!(
                        "focus_window_then_activate_fallback direct focus failed app={:?} title={:?} window_id={} error={error:#}",
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

fn focus_window_for_pid(pid: c_int, window_id: u32) -> Result<()> {
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
        let Some(candidate_window_id) = copy_ax_window_id(window)? else {
            continue;
        };
        if candidate_window_id != window_id {
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
        bundle_root_from_executable_path, login_item_target_from_executable_path,
        requires_clipboard_paste,
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

        assert_eq!(target.path, "/Applications/Runx.app");
        assert_eq!(target.name, "Runx");
    }

    #[test]
    fn login_item_target_is_unavailable_for_non_bundled_executable() {
        assert!(login_item_target_from_executable_path(Path::new("/usr/bin/ssh")).is_none());
    }
}
