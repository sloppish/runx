//! Provider for currently open windows.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use anyhow::{Result, bail};
use core_foundation::{
    base::{CFType, TCFType},
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_foundation_sys::string::CFStringRef;
use core_graphics::window::{
    copy_window_info, kCGNullWindowID, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
    kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowNumber, kCGWindowOwnerName,
    kCGWindowOwnerPID,
};

use crate::{
    icons::IconCache,
    macos,
    scoring::fuzzy_score,
    types::{Action, SearchItem},
};

/// Searches the current on-screen window list with a launcher-session snapshot.
pub struct WindowsProvider {
    icons: Arc<IconCache>,
    include_other_desktops: bool,
    session: Mutex<WindowSession>,
}

#[derive(Clone)]
struct WindowRecord {
    title: String,
    owner: String,
    pid: i64,
    window_id: u32,
    z_index: usize,
}

#[derive(Default)]
struct WindowSession {
    active: bool,
    windows: Option<Vec<WindowRecord>>,
}

struct AccessibilityWindowRecord {
    title: String,
    focusable: bool,
}

const RUNX_OWNER: &str = "Runx";
const RUNX_LAUNCHER_TITLE: &str = "Runx";

impl WindowsProvider {
    /// Creates a window provider backed by the shared icon cache.
    pub fn new(icons: Arc<IconCache>, include_other_desktops: bool) -> Self {
        Self {
            icons,
            include_other_desktops,
            session: Mutex::new(WindowSession::default()),
        }
    }

    /// Starts a new launcher-visible session.
    pub fn begin_session(&self) {
        let mut session = lock_or_recover(&self.session);
        session.active = true;
        session.windows = None;
    }

    /// Ends the current launcher-visible session and clears the cached snapshot.
    pub fn end_session(&self) {
        let mut session = lock_or_recover(&self.session);
        session.active = false;
        session.windows = None;
        self.icons.clear_process_bundle_cache();
    }

    /// Returns the highest-scoring visible windows for the current query.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchItem>> {
        let windows = match self.session_windows()? {
            Some(windows) => windows,
            None => return Ok(Vec::new()),
        };

        let ranked = rank_windows(&windows, query, limit);
        Ok(ranked
            .into_iter()
            .map(|(score, window)| SearchItem {
                id: format!("window:{}:{}", window.pid, window.window_id),
                provider: "windows".to_owned(),
                badge: "WIN".to_owned(),
                icon: self.icons.icon_for_pid(window.pid),
                title: window.title.clone(),
                subtitle: window.owner.clone(),
                compact: false,
                raw_score: score,
                action: Action::FocusWindow {
                    app_name: window.owner,
                    window_title: window.title,
                    window_id: window.window_id,
                    pid: window.pid,
                },
            })
            .collect())
    }

    fn session_windows(&self) -> Result<Option<Vec<WindowRecord>>> {
        {
            let session = lock_or_recover(&self.session);
            if !session.active {
                return Ok(None);
            }
            if let Some(windows) = &session.windows {
                return Ok(Some(windows.clone()));
            }
        }

        if !self.ensure_required_permissions(true)? {
            return Ok(None);
        }

        let windows = read_windows(self.include_other_desktops);
        let mut session = lock_or_recover(&self.session);
        if session.active {
            session.windows = Some(windows.clone());
        }
        Ok(Some(windows))
    }

    fn ensure_required_permissions(&self, prompt: bool) -> Result<bool> {
        if !macos::ensure_accessibility_trusted(prompt) {
            return Ok(false);
        }

        if self.include_other_desktops && !macos::ensure_screen_recording_trusted(prompt) {
            bail!(
                "Searching windows from other desktops requires Screen Recording permission. Enable Runx in System Settings > Privacy & Security > Screen & System Audio Recording, then restart Runx."
            );
        }

        Ok(true)
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn read_windows(include_other_desktops: bool) -> Vec<WindowRecord> {
    let onscreen = read_window_entries(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        true,
    );

    if !include_other_desktops {
        return assign_z_indices(onscreen);
    }

    let all_windows = read_window_entries(kCGWindowListExcludeDesktopElements, false);
    merge_window_orders(onscreen, all_windows)
}

fn read_window_entries(list_options: u32, fallback_empty_titles: bool) -> Vec<WindowRecord> {
    let Some(array) = copy_window_info(list_options, kCGNullWindowID) else {
        return Vec::new();
    };

    let mut raw_windows = Vec::new();

    for raw_value in array.get_all_values() {
        let dict = unsafe { CFDictionary::<CFString, CFType>::wrap_under_get_rule(raw_value as _) };

        let layer = get_number(&dict, unsafe { kCGWindowLayer }).unwrap_or_default();
        if layer != 0 {
            continue;
        }

        let owner = get_string(&dict, unsafe { kCGWindowOwnerName }).unwrap_or_default();
        let title = get_string(&dict, unsafe { kCGWindowName }).unwrap_or_default();
        let pid = get_number(&dict, unsafe { kCGWindowOwnerPID }).unwrap_or_default();
        let window_id = get_number(&dict, unsafe { kCGWindowNumber }).unwrap_or_default() as u32;

        if owner.is_empty() || pid == 0 || window_id == 0 {
            continue;
        }

        if matches!(
            owner.as_str(),
            "Window Server" | "Dock" | "Control Centre" | "Control Center"
        ) {
            continue;
        }

        raw_windows.push(WindowRecord {
            title,
            owner,
            pid,
            window_id,
            z_index: 0,
        });
    }

    let ax_windows = accessibility_windows_by_window(raw_windows.as_slice());
    let regular_app_pids = regular_app_pids(raw_windows.as_slice());
    apply_accessibility_titles(
        raw_windows,
        &ax_windows,
        &regular_app_pids,
        fallback_empty_titles,
    )
}

fn accessibility_windows_by_window(
    windows: &[WindowRecord],
) -> HashMap<(i64, u32), AccessibilityWindowRecord> {
    let mut output = HashMap::new();
    let pids: HashSet<i64> = windows.iter().map(|window| window.pid).collect();
    for pid in pids {
        for window in macos::accessibility_windows_for_pid(pid) {
            output.insert(
                (pid, window.window_id),
                AccessibilityWindowRecord {
                    title: window.title,
                    focusable: is_focusable_accessibility_subrole(&window.subrole),
                },
            );
        }
    }
    output
}

fn regular_app_pids(windows: &[WindowRecord]) -> HashSet<i64> {
    let pids: HashSet<i64> = windows.iter().map(|window| window.pid).collect();
    pids.into_iter()
        .filter(|pid| macos::running_application_is_regular(*pid))
        .collect()
}

fn apply_accessibility_titles(
    raw_windows: Vec<WindowRecord>,
    ax_windows: &HashMap<(i64, u32), AccessibilityWindowRecord>,
    regular_app_pids: &HashSet<i64>,
    fallback_empty_titles: bool,
) -> Vec<WindowRecord> {
    let mut windows = Vec::with_capacity(raw_windows.len());
    for mut window in raw_windows {
        let ax_window = ax_windows.get(&(window.pid, window.window_id));
        let is_regular_app = regular_app_pids.contains(&window.pid);

        if let Some(ax_window) = ax_window
            && !ax_window.title.is_empty()
        {
            window.title.clone_from(&ax_window.title);
        }

        if !is_regular_app && !matches!(ax_window, Some(ax_window) if ax_window.focusable) {
            continue;
        }

        if window.title.is_empty() {
            if !fallback_empty_titles || !is_regular_app {
                continue;
            }
            window.title.clone_from(&window.owner);
        }
        if is_runx_launcher_window(&window) {
            continue;
        }
        windows.push(window);
    }

    windows
}

fn is_runx_launcher_window(window: &WindowRecord) -> bool {
    window.owner == RUNX_OWNER && window.title == RUNX_LAUNCHER_TITLE
}

fn is_focusable_accessibility_subrole(subrole: &str) -> bool {
    matches!(subrole, "AXStandardWindow" | "AXDialog")
}

fn assign_z_indices(mut windows: Vec<WindowRecord>) -> Vec<WindowRecord> {
    for (index, window) in windows.iter_mut().enumerate() {
        window.z_index = index;
    }
    windows
}

fn merge_window_orders(
    onscreen_windows: Vec<WindowRecord>,
    all_windows: Vec<WindowRecord>,
) -> Vec<WindowRecord> {
    let mut merged = Vec::with_capacity(all_windows.len().max(onscreen_windows.len()));
    let mut seen = HashSet::with_capacity(all_windows.len().max(onscreen_windows.len()));

    for window in onscreen_windows {
        if seen.insert(window.window_id) {
            merged.push(window);
        }
    }

    for window in all_windows {
        if seen.insert(window.window_id) {
            merged.push(window);
        }
    }

    assign_z_indices(merged)
}

fn get_string(dict: &CFDictionary<CFString, CFType>, key: CFStringRef) -> Option<String> {
    let key = unsafe { CFString::wrap_under_get_rule(key) };
    dict.find(&key)
        .and_then(|value| value.downcast::<CFString>())
        .map(|value| value.to_string())
}

fn get_number(dict: &CFDictionary<CFString, CFType>, key: CFStringRef) -> Option<i64> {
    let key = unsafe { CFString::wrap_under_get_rule(key) };
    dict.find(&key)
        .and_then(|value| value.downcast::<CFNumber>())
        .and_then(|value| value.to_i64())
}

fn rank_windows(windows: &[WindowRecord], query: &str, limit: usize) -> Vec<(i64, WindowRecord)> {
    let query = query.trim();
    let empty_query = query.is_empty();
    let mut items = Vec::new();

    for window in windows.iter() {
        if window.z_index == 0 {
            continue;
        }

        let score = if empty_query {
            empty_query_score(window)
        } else {
            let combined = format!("{} {}", window.title, window.owner);
            fuzzy_score(&combined, query) + (fuzzy_score(&window.title, query) / 2)
        };

        if score <= 0 {
            continue;
        }

        items.push((score, window.clone()));
    }

    items.sort_by_key(|item| std::cmp::Reverse(item.0));
    items.truncate(limit);
    items
}

fn empty_query_score(window: &WindowRecord) -> i64 {
    10_000 - (window.z_index as i64 * 15)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::{
        AccessibilityWindowRecord, WindowRecord, apply_accessibility_titles, empty_query_score,
        merge_window_orders, rank_windows,
    };

    fn window(window_id: u32, z_index: usize) -> WindowRecord {
        WindowRecord {
            title: "title".to_owned(),
            owner: "owner".to_owned(),
            pid: 1,
            window_id,
            z_index,
        }
    }

    fn regular_app_pids() -> HashSet<i64> {
        HashSet::from([1])
    }

    fn ax_window(title: &str) -> AccessibilityWindowRecord {
        AccessibilityWindowRecord {
            title: title.to_owned(),
            focusable: true,
        }
    }

    fn non_focusable_ax_window(title: &str) -> AccessibilityWindowRecord {
        AccessibilityWindowRecord {
            title: title.to_owned(),
            focusable: false,
        }
    }

    #[test]
    fn merge_prefers_onscreen_windows_over_other_desktops() {
        let merged = merge_window_orders(
            vec![window(10, 0), window(20, 0)],
            vec![window(10, 0), window(30, 0), window(20, 0), window(40, 0)],
        );

        let ids: Vec<u32> = merged.into_iter().map(|window| window.window_id).collect();
        assert_eq!(ids, vec![10, 20, 30, 40]);
    }

    #[test]
    fn merge_keeps_onscreen_z_order_instead_of_all_windows_grouping() {
        let merged = merge_window_orders(
            vec![window(200, 0), window(100, 0)],
            vec![
                window(200, 0),
                window(201, 0),
                window(202, 0),
                window(100, 0),
            ],
        );

        let ids: Vec<u32> = merged.into_iter().map(|window| window.window_id).collect();
        assert_eq!(ids, vec![200, 100, 201, 202]);
    }

    #[test]
    fn empty_query_keeps_z_order_within_merged_order() {
        let front = window(1, 1);
        let back = window(2, 5);

        assert!(empty_query_score(&front) > empty_query_score(&back));
    }

    #[test]
    fn accessibility_title_overrides_core_graphics_title() {
        let mut ax_titles = HashMap::new();
        ax_titles.insert((1, 10), ax_window("Accessibility title"));

        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: "CoreGraphics title".to_owned(),
                owner: "Example".to_owned(),
                pid: 1,
                window_id: 10,
                z_index: 0,
            }],
            &ax_titles,
            &regular_app_pids(),
            true,
        );

        assert_eq!(windows[0].title, "Accessibility title");
    }

    #[test]
    fn missing_window_title_falls_back_to_owner() {
        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: String::new(),
                owner: "Example".to_owned(),
                pid: 1,
                window_id: 10,
                z_index: 0,
            }],
            &HashMap::new(),
            &regular_app_pids(),
            true,
        );

        assert_eq!(windows[0].title, "Example");
    }

    #[test]
    fn missing_window_title_without_fallback_is_dropped() {
        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: String::new(),
                owner: "Example".to_owned(),
                pid: 1,
                window_id: 10,
                z_index: 0,
            }],
            &HashMap::new(),
            &regular_app_pids(),
            false,
        );

        assert!(windows.is_empty());
    }

    #[test]
    fn accessibility_title_keeps_window_without_fallback() {
        let mut ax_titles = HashMap::new();
        ax_titles.insert((1, 10), ax_window("Accessibility title"));

        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: String::new(),
                owner: "Example".to_owned(),
                pid: 1,
                window_id: 10,
                z_index: 0,
            }],
            &ax_titles,
            &HashSet::new(),
            false,
        );

        assert_eq!(windows[0].title, "Accessibility title");
    }

    #[test]
    fn non_regular_non_accessibility_window_is_dropped() {
        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: "Menu bar surface".to_owned(),
                owner: "Accessory".to_owned(),
                pid: 2,
                window_id: 10,
                z_index: 0,
            }],
            &HashMap::new(),
            &regular_app_pids(),
            true,
        );

        assert!(windows.is_empty());
    }

    #[test]
    fn non_regular_non_focusable_accessibility_window_is_dropped() {
        let mut ax_windows = HashMap::new();
        ax_windows.insert((2, 10), non_focusable_ax_window(""));

        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: String::new(),
                owner: "Accessory".to_owned(),
                pid: 2,
                window_id: 10,
                z_index: 0,
            }],
            &ax_windows,
            &regular_app_pids(),
            true,
        );

        assert!(windows.is_empty());
    }

    #[test]
    fn non_regular_titleless_accessibility_window_does_not_fallback_to_owner() {
        let mut ax_windows = HashMap::new();
        ax_windows.insert((2, 10), ax_window(""));

        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: String::new(),
                owner: "Accessory".to_owned(),
                pid: 2,
                window_id: 10,
                z_index: 0,
            }],
            &ax_windows,
            &regular_app_pids(),
            true,
        );

        assert!(windows.is_empty());
    }

    #[test]
    fn non_regular_focusable_titled_accessibility_window_is_kept() {
        let mut ax_windows = HashMap::new();
        ax_windows.insert((2, 10), ax_window("Preferences"));

        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: String::new(),
                owner: "Accessory".to_owned(),
                pid: 2,
                window_id: 10,
                z_index: 0,
            }],
            &ax_windows,
            &regular_app_pids(),
            true,
        );

        assert_eq!(windows[0].title, "Preferences");
    }

    #[test]
    fn regular_non_accessibility_window_is_kept() {
        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: "Other desktop window".to_owned(),
                owner: "Regular".to_owned(),
                pid: 1,
                window_id: 10,
                z_index: 0,
            }],
            &HashMap::new(),
            &regular_app_pids(),
            false,
        );

        assert_eq!(windows[0].title, "Other desktop window");
    }

    #[test]
    fn runx_launcher_window_is_hidden() {
        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: "Runx".to_owned(),
                owner: "Runx".to_owned(),
                pid: 1,
                window_id: 10,
                z_index: 0,
            }],
            &HashMap::new(),
            &regular_app_pids(),
            true,
        );

        assert!(windows.is_empty());
    }

    #[test]
    fn runx_settings_window_is_kept() {
        let windows = apply_accessibility_titles(
            vec![WindowRecord {
                title: "Runx Settings".to_owned(),
                owner: "Runx".to_owned(),
                pid: 1,
                window_id: 10,
                z_index: 0,
            }],
            &HashMap::new(),
            &regular_app_pids(),
            true,
        );

        assert_eq!(windows[0].title, "Runx Settings");
    }

    fn named_window(title: &str, owner: &str, window_id: u32, z_index: usize) -> WindowRecord {
        WindowRecord {
            title: title.to_owned(),
            owner: owner.to_owned(),
            pid: 1,
            window_id,
            z_index,
        }
    }

    #[test]
    fn empty_query_excludes_topmost_window() {
        let windows = vec![
            named_window("Front", "App", 1, 0),
            named_window("Middle", "App", 2, 1),
            named_window("Back", "App", 3, 2),
        ];

        let results = rank_windows(&windows, "", 10);
        let titles: Vec<&str> = results.iter().map(|(_, w)| w.title.as_str()).collect();

        assert!(!titles.contains(&"Front"));
        assert!(titles.contains(&"Middle"));
        assert!(titles.contains(&"Back"));
    }

    #[test]
    fn non_empty_query_excludes_topmost_window_not_best_match() {
        let windows = vec![
            named_window("Terminal", "Terminal", 1, 0),
            named_window("Terminal — ssh", "Terminal", 2, 1),
            named_window("Finder", "Finder", 3, 2),
        ];

        let results = rank_windows(&windows, "Terminal", 10);
        let titles: Vec<&str> = results.iter().map(|(_, w)| w.title.as_str()).collect();

        assert!(!titles.contains(&"Terminal"));
        assert!(titles.contains(&"Terminal — ssh"));
    }

    #[test]
    fn non_topmost_matching_window_is_kept() {
        let windows = vec![
            named_window("Finder", "Finder", 1, 0),
            named_window("Safari", "Safari", 2, 1),
        ];

        let results = rank_windows(&windows, "Safari", 10);
        let titles: Vec<&str> = results.iter().map(|(_, w)| w.title.as_str()).collect();

        assert!(titles.contains(&"Safari"));
    }

    #[test]
    fn only_topmost_matching_window_returns_empty() {
        let windows = vec![
            named_window("Safari", "Safari", 1, 0),
            named_window("Finder", "Finder", 2, 1),
        ];

        let results = rank_windows(&windows, "Safari", 10);
        let titles: Vec<&str> = results.iter().map(|(_, w)| w.title.as_str()).collect();

        assert!(titles.is_empty());
    }
}
