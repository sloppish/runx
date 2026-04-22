//! Provider for currently open windows.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use anyhow::Result;
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

impl WindowsProvider {
    /// Creates a window provider backed by the shared icon cache.
    pub fn new(icons: Arc<IconCache>, include_other_desktops: bool) -> Self {
        Self {
            icons,
            include_other_desktops,
            session: Mutex::new(WindowSession::default()),
        }
    }

    /// Starts a new launcher-visible session and captures a fresh snapshot when permitted.
    pub fn begin_session(&self) {
        let windows =
            macos::has_screen_capture_access().then(|| read_windows(self.include_other_desktops));
        let mut session = lock_or_recover(&self.session);
        session.active = true;
        session.windows = windows;
    }

    /// Ends the current launcher-visible session and clears the cached snapshot.
    pub fn end_session(&self) {
        let mut session = lock_or_recover(&self.session);
        session.active = false;
        session.windows = None;
    }

    /// Returns the highest-scoring visible windows for the current query.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchItem>> {
        let windows = match self.session_windows() {
            Some(windows) => windows,
            None => return Ok(Vec::new()),
        };

        let query = query.trim();
        let empty_query = query.is_empty();
        let mut items = Vec::new();

        for window in windows.iter() {
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
        if empty_query && !items.is_empty() {
            items.remove(0);
        }
        items.truncate(limit);
        Ok(items
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
                },
            })
            .collect())
    }

    fn session_windows(&self) -> Option<Vec<WindowRecord>> {
        {
            let session = lock_or_recover(&self.session);
            if !session.active {
                return None;
            }
            if let Some(windows) = &session.windows {
                return Some(windows.clone());
            }
        }

        if !macos::request_screen_capture_access_once() {
            return None;
        }

        let windows = read_windows(self.include_other_desktops);
        let mut session = lock_or_recover(&self.session);
        if session.active {
            session.windows = Some(windows.clone());
        }
        Some(windows)
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn read_windows(include_other_desktops: bool) -> Vec<WindowRecord> {
    let onscreen =
        read_window_entries(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements);

    if !include_other_desktops {
        return assign_z_indices(onscreen);
    }

    let all_windows = read_window_entries(kCGWindowListExcludeDesktopElements);
    merge_window_orders(onscreen, all_windows)
}

fn read_window_entries(list_options: u32) -> Vec<WindowRecord> {
    let Some(array) = copy_window_info(list_options, kCGNullWindowID) else {
        return Vec::new();
    };

    let mut windows = Vec::new();

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

        if owner.is_empty() || title.is_empty() {
            continue;
        }

        if matches!(
            owner.as_str(),
            "Window Server" | "Dock" | "Control Centre" | "Control Center"
        ) {
            continue;
        }

        windows.push(WindowRecord {
            title,
            owner,
            pid,
            window_id,
            z_index: 0,
        });
    }

    windows
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

fn empty_query_score(window: &WindowRecord) -> i64 {
    10_000 - (window.z_index as i64 * 15)
}

#[cfg(test)]
mod tests {
    use super::{WindowRecord, empty_query_score, merge_window_orders};

    fn window(window_id: u32, z_index: usize) -> WindowRecord {
        WindowRecord {
            title: "title".to_owned(),
            owner: "owner".to_owned(),
            pid: 1,
            window_id,
            z_index,
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
}
