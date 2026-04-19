use std::{sync::Arc, time::{Duration, Instant}};

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
    kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowOwnerName, kCGWindowOwnerPID,
};

use crate::{
    icons::IconCache,
    scoring::fuzzy_score,
    types::{Action, SearchItem},
};

pub struct WindowsProvider {
    cache: std::sync::Mutex<WindowCache>,
    icons: Arc<IconCache>,
}

#[derive(Default)]
struct WindowCache {
    updated_at: Option<Instant>,
    items: Vec<WindowRecord>,
}

#[derive(Clone)]
struct WindowRecord {
    title: String,
    owner: String,
    pid: i64,
    z_index: usize,
}

impl WindowsProvider {
    pub fn new(icons: Arc<IconCache>) -> Self {
        Self {
            cache: std::sync::Mutex::new(WindowCache::default()),
            icons,
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchItem>> {
        let windows = self.snapshot()?;
        let query = query.trim();
        let mut items = Vec::new();

        for window in windows.iter() {
            let score = if query.is_empty() {
                10_000 - (window.z_index as i64 * 15)
            } else {
                let combined = format!("{} {}", window.title, window.owner);
                fuzzy_score(&combined, query) + (fuzzy_score(&window.title, query) / 2)
            };

            if score <= 0 {
                continue;
            }

            items.push((score, window.clone()));
        }

        items.sort_by(|left, right| right.0.cmp(&left.0));
        items.truncate(limit);
        Ok(items
            .into_iter()
            .map(|(score, window)| SearchItem {
                id: format!("window:{}:{}:{}", window.pid, window.owner, window.title),
                provider: "windows".to_owned(),
                badge: "WIN".to_owned(),
                icon: self.icons.icon_for_pid(window.pid),
                title: window.title.clone(),
                subtitle: window.owner.clone(),
                raw_score: score,
                action: Action::FocusWindow {
                    app_name: window.owner,
                    window_title: window.title,
                },
            })
            .collect())
    }

    fn snapshot(&self) -> Result<Vec<WindowRecord>> {
        let mut cache = self.cache.lock().expect("windows cache poisoned");
        if let Some(updated_at) = cache.updated_at {
            if updated_at.elapsed() < Duration::from_millis(900) {
                return Ok(cache.items.clone());
            }
        }

        let fresh = read_windows();
        cache.updated_at = Some(Instant::now());
        cache.items = fresh.clone();
        Ok(fresh)
    }
}

fn read_windows() -> Vec<WindowRecord> {
    let Some(array) = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return Vec::new();
    };

    let mut windows = Vec::new();

    for (index, raw_value) in array.get_all_values().into_iter().enumerate() {
        let dict = unsafe { CFDictionary::<CFString, CFType>::wrap_under_get_rule(raw_value as _) };

        let layer = get_number(&dict, unsafe { kCGWindowLayer }).unwrap_or_default();
        if layer != 0 {
            continue;
        }

        let owner = get_string(&dict, unsafe { kCGWindowOwnerName }).unwrap_or_default();
        let title = get_string(&dict, unsafe { kCGWindowName }).unwrap_or_default();
        let pid = get_number(&dict, unsafe { kCGWindowOwnerPID }).unwrap_or_default();

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
            z_index: index,
        });
    }

    windows
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
