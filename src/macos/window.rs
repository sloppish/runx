use std::ffi::c_int;

use anyhow::{Result, bail};
use core_foundation::base::{CFType, TCFType};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::{
    display::CGDisplay,
    event::CGEvent,
    event_source::{CGEventSource, CGEventSourceStateID},
    window::{copy_window_info, kCGNullWindowID, kCGWindowListOptionOnScreenOnly, kCGWindowNumber},
};
use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_app_kit::{
    NSWindow, NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, ns_string};
use tao::{platform::macos::WindowExtMacOS, window::Window};

use tracing::debug;

use super::{
    apps::{
        AppActivationMode, activate_running_application_by_pid,
        activate_running_application_by_pid_with_mode,
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

const LAUNCHER_TOP_OFFSET_DIVISOR: f64 = 3.2;

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
pub fn configure_launcher_panel(window: &Window, show_animation: bool) {
    let window = ns_window(window);
    let mut style_mask = window.styleMask();
    style_mask.insert(NSWindowStyleMask::NonactivatingPanel);
    window.setStyleMask(style_mask);
    window.setHidesOnDeactivate(false);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::MoveToActiveSpace
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    set_window_animation(window, show_animation);
}

/// Enables or disables the macOS show/hide zoom animation on the launcher window.
pub fn set_launcher_animation(window: &Window, show_animation: bool) {
    set_window_animation(ns_window(window), show_animation);
}

fn set_window_animation(window: &NSWindow, show_animation: bool) {
    let behavior = if show_animation {
        NSWindowAnimationBehavior::Default
    } else {
        NSWindowAnimationBehavior::None
    };
    window.setAnimationBehavior(behavior);
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

/// Returns the logical size of the AppKit screen for a CoreGraphics display ID.
pub fn display_logical_size(display_id: u32) -> Option<(f64, f64)> {
    let screen = screen_for_display_id(display_id)?;
    let frame = screen.frame();
    Some((frame.size.width, frame.size.height))
}

/// Positions the launcher panel on the selected AppKit screen.
pub fn position_launcher_panel(window: &Window, display_id: u32, width: f64, height: f64) -> bool {
    let Some(screen) = screen_for_display_id(display_id) else {
        return false;
    };

    let frame = screen.frame();
    let top_left = launcher_panel_top_left(frame, width, height);
    let window = ns_window(window);
    window.setContentSize(NSSize::new(width, height));
    window.setFrameTopLeftPoint(top_left);
    true
}

/// Focuses a single window. When the window is on the current Space, uses
/// SkyLight to raise just that window without pulling the entire app forward.
/// When on a different Space, activates the app first (triggering a Space
/// switch), then focuses the exact window.
pub fn focus_window(
    app_name: &str,
    window_title: &str,
    window_id: u32,
    pid: i64,
) -> Result<Option<String>> {
    let pid = pid as c_int;
    let on_current_space = is_window_on_screen(window_id);

    if !on_current_space {
        activate_running_application_by_pid(pid)?;
    }

    if ensure_accessibility_trusted(true) {
        match focus_window_for_pid(pid, window_id) {
            Ok(_) => return Ok(Some(format!("Focused {}", window_title))),
            Err(error) => {
                debug!(
                    app = ?app_name,
                    title = ?window_title,
                    window_id,
                    error = %format!("{error:#}"),
                    "focus_window direct focus failed"
                );
            }
        }
    } else {
        open_accessibility_settings();
    }

    if on_current_space {
        activate_running_application_by_pid(pid)?;
    }
    Ok(Some(format!("Activated {}", app_name)))
}

pub fn focus_window_and_activate_all_windows(
    app_name: &str,
    window_title: &str,
    window_id: u32,
    pid: i64,
) -> Result<Option<String>> {
    let pid = pid as c_int;
    activate_running_application_by_pid_with_mode(pid, AppActivationMode::AllWindows)?;

    if ensure_accessibility_trusted(true) {
        match focus_window_for_pid(pid, window_id) {
            Ok(_) => {
                return Ok(Some(format!(
                    "Focused {} and activated all {} windows",
                    window_title, app_name
                )));
            }
            Err(error) => {
                debug!(
                    app = ?app_name,
                    title = ?window_title,
                    window_id,
                    error = %format!("{error:#}"),
                    "focus_window_and_activate_all_windows focus failed"
                );
            }
        }
    } else {
        open_accessibility_settings();
        return Ok(Some(format!(
            "Activated all {} windows (window focus requires Accessibility permission)",
            app_name
        )));
    }

    Ok(Some(format!("Activated all {} windows", app_name)))
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

fn focus_window_for_pid(pid: c_int, window_id: u32) -> Result<()> {
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
                debug!(
                    pid,
                    error = %format!("{error:#}"),
                    "focus_window skipped inaccessible AX window"
                );
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

fn focus_ax_window(pid: c_int, window_id: u32, window: AXUIElementRef) -> Result<()> {
    let mut did_focus = false;

    if focus_cg_window(pid, window_id).is_ok() {
        did_focus = true;
    }
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
        return Ok(());
    }

    bail!("window exists but does not expose focus actions")
}

fn screen_for_display_id(display_id: u32) -> Option<objc2::rc::Retained<NSScreen>> {
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    for index in 0..screens.count() {
        let screen = screens.objectAtIndex(index);
        if screen_display_id(&screen) == display_id {
            return Some(screen);
        }
    }
    None
}

fn screen_display_id(screen: &NSScreen) -> u32 {
    screen
        .deviceDescription()
        .objectForKey(ns_string!("NSScreenNumber"))
        .and_then(|value| value.downcast::<NSNumber>().ok())
        .map_or(0, |value| value.as_u32())
}

fn launcher_panel_top_left(screen_frame: NSRect, window_width: f64, window_height: f64) -> NSPoint {
    let x = screen_frame.origin.x + (screen_frame.size.width - window_width) / 2.0;
    let top_offset = (screen_frame.size.height - window_height) / LAUNCHER_TOP_OFFSET_DIVISOR;
    let y = screen_frame.origin.y + screen_frame.size.height - top_offset;
    NSPoint::new(x.round(), y.round())
}

fn is_window_on_screen(window_id: u32) -> bool {
    let Some(array) = copy_window_info(kCGWindowListOptionOnScreenOnly, kCGNullWindowID) else {
        return false;
    };
    let key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };
    array.get_all_values().iter().any(|raw_value| {
        let dict = unsafe {
            core_foundation::dictionary::CFDictionary::<CFString, CFType>::wrap_under_get_rule(
                *raw_value as _,
            )
        };
        dict.find(&key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|v| v.to_i64())
            .is_some_and(|wid| wid as u32 == window_id)
    })
}

fn ns_window(window: &Window) -> &NSWindow {
    unsafe { &*(window.ns_window() as *mut NSWindow) }
}

#[cfg(test)]
mod tests {
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    use super::launcher_panel_top_left;

    #[test]
    fn launcher_panel_top_left_uses_right_hand_screen_frame() {
        let screen_frame = NSRect::new(NSPoint::new(1728.0, 0.0), NSSize::new(2560.0, 1440.0));

        let top_left = launcher_panel_top_left(screen_frame, 960.0, 500.0);

        assert_eq!(top_left.x, 2528.0);
        assert_eq!(top_left.y, 1146.0);
    }

    #[test]
    fn launcher_panel_top_left_uses_lower_screen_frame() {
        let screen_frame = NSRect::new(NSPoint::new(0.0, -1440.0), NSSize::new(2560.0, 1440.0));

        let top_left = launcher_panel_top_left(screen_frame, 960.0, 500.0);

        assert_eq!(top_left.x, 800.0);
        assert_eq!(top_left.y, -294.0);
    }
}
