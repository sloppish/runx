use std::ffi::c_int;

use anyhow::{Context, Result, bail};
use core_foundation::base::TCFType;
use core_graphics::{
    display::CGDisplay,
    event::CGEvent,
    event_source::{CGEventSource, CGEventSourceStateID},
};
use objc2_app_kit::{NSWindow, NSWindowStyleMask};
use tao::{platform::macos::WindowExtMacOS, window::Window};

use crate::debug_log;

use super::{
    apps::{
        AppActivationMode, activate_running_application_by_pid,
        activate_running_application_by_pid_with_mode, open_named_application,
        running_application_pid_by_name,
    },
    permissions::{ensure_accessibility_trusted, open_accessibility_settings},
    private_apis::{
        AX_MESSAGING_TIMEOUT, AXUIElementRef, OwnedAxElement, ax_error_message,
        ax_focused_attribute, ax_main_attribute, ax_raise_action, ax_subrole_attribute,
        ax_title_attribute, ax_windows_attribute, copy_ax_string, copy_ax_window_id,
        focus_cg_window, perform_ax_action, remote_ax_window_for_pid, set_ax_bool_attribute,
    },
    types::{AccessibilityWindow, CursorDisplayLocation},
};

#[derive(Debug, Clone, Copy)]
struct ExactWindowFocus {
    activated_app: bool,
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

/// Returns Accessibility-visible windows for a running application.
pub fn accessibility_windows_for_pid(pid: i64) -> Vec<AccessibilityWindow> {
    let Some(app_element) = OwnedAxElement::application(pid as c_int) else {
        return Vec::new();
    };
    let _ = app_element.set_messaging_timeout(AX_MESSAGING_TIMEOUT);

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

fn focus_window_for_pid(pid: c_int, window_id: u32) -> Result<ExactWindowFocus> {
    let app_element = OwnedAxElement::application(pid)
        .ok_or_else(|| anyhow::anyhow!("failed to create accessibility handle for pid {pid}"))?;
    let _ = app_element.set_messaging_timeout(AX_MESSAGING_TIMEOUT);

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

fn ns_window(window: &Window) -> &NSWindow {
    unsafe { &*(window.ns_window() as *mut NSWindow) }
}
