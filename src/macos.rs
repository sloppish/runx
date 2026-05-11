//! macOS-specific shell-outs and permission-sensitive integrations.
//!
//! If something talks to `open`, AppKit, Accessibility, or Quartz events
//! trust, it belongs here rather than leaking platform details into the rest of
//! the launcher.

mod apps;
mod clipboard;
mod login_items;
mod permissions;
mod private_apis;
mod process;
mod types;
mod window;

pub use apps::{
    capture_frontmost_app, open_application, open_settings, regular_pids_from_running_applications,
};
pub use clipboard::{copy_text_to_clipboard, read_clipboard_text, type_text_into_previous_app};
pub use login_items::{
    current_app_login_item_enabled, current_app_supports_login_item, toggle_current_app_login_item,
};
pub use permissions::{ensure_accessibility_trusted, ensure_screen_recording_trusted};
pub use types::FrontmostApp;
pub use window::{
    accessibility_windows_for_pid, configure_launcher_panel, cursor_display_location,
    display_logical_size, focus_launcher_panel, focus_window,
    focus_window_and_activate_all_windows, option_key_pressed, position_launcher_panel,
    set_launcher_animation, show_launcher_panel,
};

#[cfg(test)]
use apps::bundle_root_from_executable_path;
#[cfg(test)]
use clipboard::requires_clipboard_paste;
#[cfg(test)]
use login_items::{
    LOGIN_AGENT_LABEL, login_agent_plist_matches_target, login_agent_plist_value,
    login_item_target_from_executable_path,
};

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
