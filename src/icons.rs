//! Icon extraction and caching for search results.
//!
//! Providers ask this module for icons so they can stay focused on search
//! semantics instead of plist parsing, process inspection, and PNG rendering.

mod bundle;
mod protocol;
mod render;
mod worker;

use std::{
    borrow::Cow,
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use directories::BaseDirs;
use tao::event_loop::EventLoopProxy;
use wry::http::{
    Request, Response, StatusCode,
    header::{CACHE_CONTROL, CONTENT_TYPE},
};

use crate::{macos, types::AppEvent};
use tracing::debug;

use self::{
    bundle::{cache_key_for_bundle, find_bundle_icon_source, process_bundle_path},
    protocol::{icon_key_from_request_path, icon_protocol_url, response_with_status},
    render::render_bundle_icon_to_webp,
    worker::RenderLimiter,
};

const SYSTEM_SETTINGS_APP_CANDIDATES: [&str; 2] = [
    "/System/Applications/System Settings.app",
    "/System/Applications/System Preferences.app",
];
pub(super) const ICON_CACHE_FORMAT_VERSION: &str = "webp-v1";
pub(super) const ICON_RENDER_SIZE: u32 = 64;
const ICON_DISK_CACHE_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const MAX_IN_MEMORY_ICON_ENTRIES: usize = 2048;
const MAX_ICON_RENDER_WORKERS: usize = 2;

/// In-memory and on-disk cache for bundle and process icons.
pub struct IconCache {
    cache_dir: PathBuf,
    icons: Arc<Mutex<IconMemoryCache>>,
    process_bundles: Mutex<HashMap<i64, Option<PathBuf>>>,
    render_limiter: Arc<RenderLimiter>,
    proxy: Option<EventLoopProxy<AppEvent>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum IconState {
    Ready(String),
    Pending,
    Missing,
}

struct IconMemoryCache {
    entries: HashMap<String, IconState>,
    lru: VecDeque<String>,
    capacity: usize,
}

impl IconMemoryCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            lru: VecDeque::new(),
            capacity,
        }
    }

    fn get_cloned(&mut self, key: &str) -> Option<IconState> {
        let state = self.entries.get(key).cloned()?;
        self.touch(key);
        Some(state)
    }

    fn reserve_pending(&mut self, key: String) -> bool {
        if self.capacity == 0 {
            return false;
        }
        if !self.ensure_capacity_for(&key) {
            return false;
        }
        self.entries.insert(key.clone(), IconState::Pending);
        self.touch(&key);
        true
    }

    fn insert(&mut self, key: String, state: IconState) {
        if self.capacity == 0 && !self.entries.contains_key(&key) {
            return;
        }
        if !self.ensure_capacity_for(&key) {
            return;
        }
        self.entries.insert(key.clone(), state);
        self.touch(&key);
    }

    fn ensure_capacity_for(&mut self, key: &str) -> bool {
        while !self.entries.contains_key(key) && self.entries.len() >= self.capacity {
            let Some(evicted_key) = self.pop_oldest_evictable() else {
                return false;
            };
            self.entries.remove(&evicted_key);
        }
        true
    }

    fn pop_oldest_evictable(&mut self) -> Option<String> {
        let index = self.lru.iter().position(|key| {
            matches!(
                self.entries.get(key),
                Some(IconState::Ready(_)) | Some(IconState::Missing)
            )
        })?;
        self.lru.remove(index)
    }

    fn touch(&mut self, key: &str) {
        self.lru.retain(|entry| entry != key);
        self.lru.push_back(key.to_owned());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IconLookupMode {
    Full,
    DirectResourceOnly,
}

impl IconLookupMode {
    fn cache_namespace(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::DirectResourceOnly => "resource",
        }
    }
}

impl IconCache {
    /// Creates the icon cache rooted in the user's cache directory.
    pub fn new(proxy: Option<EventLoopProxy<AppEvent>>) -> Result<Self> {
        let base_dirs =
            BaseDirs::new().context("could not resolve the current user's home directory")?;
        let cache_dir = base_dirs.cache_dir().join("runx/icons");
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("failed to create {}", cache_dir.display()))?;
        cleanup_stale_disk_cache_entries(&cache_dir);

        Ok(Self {
            cache_dir,
            icons: Arc::new(Mutex::new(IconMemoryCache::new(MAX_IN_MEMORY_ICON_ENTRIES))),
            process_bundles: Mutex::new(HashMap::new()),
            render_limiter: Arc::new(RenderLimiter::new()),
            proxy,
        })
    }

    /// Resolves an icon for an application bundle path and returns a protocol URL when ready.
    pub fn icon_for_bundle<P: AsRef<Path>>(&self, bundle_path: P) -> Option<String> {
        let bundle_path = bundle_path.as_ref();
        self.icon_for_bundle_with_mode(bundle_path, IconLookupMode::Full)
    }

    /// Resolves an icon for a running application's bundle identifier.
    pub fn icon_for_bundle_id(&self, bundle_id: &str) -> Option<String> {
        let bundle_id = bundle_id.trim();
        if bundle_id.is_empty() {
            return None;
        }

        let bundle_path = macos::running_applications()
            .into_iter()
            .find(|app| app.bundle_id.as_deref() == Some(bundle_id))
            .and_then(|app| app.path)?;

        self.icon_for_bundle(bundle_path)
    }

    /// Resolves an icon only when the bundle exposes a concrete icon file.
    pub fn icon_for_bundle_resource<P: AsRef<Path>>(&self, bundle_path: P) -> Option<String> {
        let bundle_path = bundle_path.as_ref();
        self.icon_for_bundle_with_mode(bundle_path, IconLookupMode::DirectResourceOnly)
    }

    fn icon_for_bundle_with_mode(
        &self,
        bundle_path: &Path,
        mode: IconLookupMode,
    ) -> Option<String> {
        let key = format!(
            "bundle:{}:{}",
            mode.cache_namespace(),
            bundle_path.display()
        );

        {
            let mut cache = lock_or_recover(&self.icons);
            match cache.get_cloned(&key) {
                Some(IconState::Ready(url)) => return Some(url),
                Some(IconState::Pending | IconState::Missing) => return None,
                None => {
                    if !cache.reserve_pending(key.clone()) {
                        return None;
                    }
                }
            }
        }

        self.resolve_or_schedule_bundle_icon(key, bundle_path, mode)
    }

    /// Resolves an icon for the owning app of a process id.
    pub fn icon_for_pid(&self, pid: i64) -> Option<String> {
        let bundle_path = {
            let cache = lock_or_recover(&self.process_bundles);
            cache.get(&pid).cloned()
        };

        let bundle_path = match bundle_path {
            Some(path) => path,
            None => {
                let resolved = process_bundle_path(pid);
                let mut cache = lock_or_recover(&self.process_bundles);
                cache.insert(pid, resolved.clone());
                resolved
            }
        }?;

        self.icon_for_bundle(bundle_path)
    }

    /// Clears process-id icon owner lookups captured during a launcher-visible session.
    pub fn clear_process_bundle_cache(&self) {
        let mut cache = lock_or_recover(&self.process_bundles);
        cache.clear();
    }

    /// Returns the System Settings app icon.
    pub fn system_settings_icon(&self) -> Option<String> {
        for bundle_path in SYSTEM_SETTINGS_APP_CANDIDATES {
            if let Some(icon) = self.icon_for_bundle(bundle_path) {
                return Some(icon);
            }
        }

        Some(system_settings_fallback_icon())
    }

    fn store_icon_state(&self, key: String, state: IconState) {
        let mut cache = lock_or_recover(&self.icons);
        cache.insert(key, state);
    }

    fn resolve_or_schedule_bundle_icon(
        &self,
        key: String,
        bundle_path: &Path,
        mode: IconLookupMode,
    ) -> Option<String> {
        if !bundle_path.exists() {
            self.store_icon_state(key, IconState::Missing);
            return None;
        }

        if mode == IconLookupMode::DirectResourceOnly
            && !matches!(find_bundle_icon_source(bundle_path), Ok(Some(_)))
        {
            self.store_icon_state(key, IconState::Missing);
            return None;
        }

        let icon_key = cache_key_for_bundle(bundle_path);
        let webp_path = self.webp_path_for_key(&icon_key);
        let url = icon_protocol_url(&icon_key);
        if webp_path.exists() {
            self.store_icon_state(key, IconState::Ready(url.clone()));
            return Some(url);
        }

        self.spawn_bundle_icon_render(key, bundle_path.to_path_buf(), webp_path, url, mode);
        None
    }

    fn spawn_bundle_icon_render(
        &self,
        key: String,
        bundle_path: PathBuf,
        webp_path: PathBuf,
        url: String,
        mode: IconLookupMode,
    ) {
        let icons = Arc::clone(&self.icons);
        let render_limiter = Arc::clone(&self.render_limiter);
        let proxy = self.proxy.clone();
        let temp_webp_path = webp_path.with_extension("tmp.webp");
        let thread_key = key.clone();

        let spawn_result = thread::Builder::new()
            .name("runx-icon-render".to_owned())
            .spawn(move || {
                let _permit = render_limiter.acquire();
                let render_result = render_bundle_icon_to_webp(&bundle_path, &temp_webp_path, mode)
                    .and_then(|()| {
                        fs::rename(&temp_webp_path, &webp_path).with_context(|| {
                            format!(
                                "failed to move {} to {}",
                                temp_webp_path.display(),
                                webp_path.display()
                            )
                        })
                    });

                let ready = render_result.is_ok();
                if let Err(error) = render_result {
                    debug!(
                        bundle = %bundle_path.display(),
                        error = %format!("{error:#}"),
                        "icon render failed"
                    );
                    let _ = fs::remove_file(&temp_webp_path);
                }

                let mut cache = lock_or_recover(&icons);
                cache.insert(
                    thread_key,
                    if ready {
                        IconState::Ready(url)
                    } else {
                        IconState::Missing
                    },
                );
                drop(cache);

                if ready && let Some(proxy) = proxy {
                    let _ = proxy.send_event(AppEvent::IconReady);
                }
            });

        if spawn_result.is_err() {
            self.store_icon_state(key, IconState::Missing);
        }
    }

    /// Resolves a `runx://localhost/icon/<hash>.webp` request into an image response.
    pub fn protocol_response(&self, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        let Some(icon_key) = icon_key_from_request_path(request.uri().path()) else {
            return response_with_status(StatusCode::NOT_FOUND, "text/plain", b"Not found");
        };

        let webp_path = self.webp_path_for_key(icon_key);
        let bytes = match fs::read(&webp_path) {
            Ok(bytes) => bytes,
            Err(_) => {
                return response_with_status(StatusCode::NOT_FOUND, "text/plain", b"Not found");
            }
        };

        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "image/webp")
            .header(CACHE_CONTROL, "public, max-age=86400")
            .body(Cow::Owned(bytes))
            .unwrap_or_else(|_| {
                response_with_status(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "text/plain",
                    b"Failed to build response",
                )
            })
    }

    fn webp_path_for_key(&self, icon_key: &str) -> PathBuf {
        self.cache_dir.join(format!(
            "{}-{}-{}px.webp",
            icon_key, ICON_CACHE_FORMAT_VERSION, ICON_RENDER_SIZE
        ))
    }
}

fn cleanup_stale_disk_cache_entries(cache_dir: &Path) {
    let Some(cutoff) = SystemTime::now().checked_sub(ICON_DISK_CACHE_TTL) else {
        return;
    };
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return;
    };
    let current_cache_suffix =
        format!("-{}-{}px.webp", ICON_CACHE_FORMAT_VERSION, ICON_RENDER_SIZE);

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }

        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_current_icon_disk_cache_file(file_name, &current_cache_suffix) {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified < cutoff {
            let _ = fs::remove_file(path);
        }
    }
}

fn is_current_icon_disk_cache_file(file_name: &str, current_cache_suffix: &str) -> bool {
    file_name.len() > current_cache_suffix.len() && file_name.ends_with(current_cache_suffix)
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn system_settings_fallback_icon() -> String {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
  <defs>
    <linearGradient id="bg" x1="20" y1="16" x2="108" y2="112" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#8fb2e6"/>
      <stop offset="1" stop-color="#4f6f9d"/>
    </linearGradient>
  </defs>
  <rect x="10" y="10" width="108" height="108" rx="28" fill="url(#bg)"/>
  <g fill="none" stroke="#f6f9ff" stroke-linecap="round" stroke-linejoin="round" stroke-width="9">
    <path d="M34 43h60"/>
    <path d="M34 64h60"/>
    <path d="M34 85h60"/>
    <circle cx="50" cy="43" r="10" fill="#f6f9ff" stroke="none"/>
    <circle cx="80" cy="64" r="10" fill="#f6f9ff" stroke="none"/>
    <circle cx="60" cy="85" r="10" fill="#f6f9ff" stroke="none"/>
  </g>
</svg>"##;

    format!(
        "data:image/svg+xml;base64,{}",
        STANDARD.encode(svg.as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        fs::{self, File, FileTimes},
        path::{Path, PathBuf},
        process,
        sync::{Arc, Mutex},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use wry::http::{Request, StatusCode};

    use super::{
        ICON_DISK_CACHE_TTL, IconCache, IconMemoryCache, IconState, MAX_IN_MEMORY_ICON_ENTRIES,
        bundle::cache_key_for_bundle, cleanup_stale_disk_cache_entries,
        protocol::icon_protocol_url, worker::RenderLimiter,
    };

    #[test]
    fn returns_ready_disk_cache_entry_without_rendering() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let bundle_path = temp_dir.join("Sample.app");
        fs::create_dir_all(&bundle_path).expect("bundle dir should be created");

        let cache = IconCache {
            cache_dir: temp_dir.clone(),
            icons: Arc::new(Mutex::new(IconMemoryCache::new(MAX_IN_MEMORY_ICON_ENTRIES))),
            process_bundles: Mutex::new(HashMap::new()),
            render_limiter: Arc::new(RenderLimiter::new()),
            proxy: None,
        };
        let icon_key = cache_key_for_bundle(&bundle_path);
        let webp_path = cache.webp_path_for_key(&icon_key);
        fs::write(&webp_path, b"RIFFxxxxWEBP").expect("cached webp should be written");

        assert_eq!(
            cache.icon_for_bundle(&bundle_path),
            Some(icon_protocol_url(&icon_key))
        );

        let request = Request::builder()
            .uri(format!("runx://localhost/icon/{icon_key}.webp"))
            .body(Vec::new())
            .expect("request should build");
        let response = cache.protocol_response(&request);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body().as_ref(), b"RIFFxxxxWEBP");

        let _ = fs::remove_file(&webp_path);
        let _ = fs::remove_dir(&bundle_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn icon_memory_cache_evicts_least_recent_ready_entry() {
        let mut cache = IconMemoryCache::new(2);
        cache.insert("a".to_owned(), IconState::Ready("a-url".to_owned()));
        cache.insert("b".to_owned(), IconState::Ready("b-url".to_owned()));

        assert_eq!(
            cache.get_cloned("a"),
            Some(IconState::Ready("a-url".to_owned()))
        );

        cache.insert("c".to_owned(), IconState::Ready("c-url".to_owned()));

        assert_eq!(cache.get_cloned("b"), None);
        assert_eq!(
            cache.get_cloned("a"),
            Some(IconState::Ready("a-url".to_owned()))
        );
        assert_eq!(
            cache.get_cloned("c"),
            Some(IconState::Ready("c-url".to_owned()))
        );
    }

    #[test]
    fn icon_memory_cache_keeps_pending_entries_bounded() {
        let mut cache = IconMemoryCache::new(1);

        assert!(cache.reserve_pending("pending".to_owned()));
        assert!(!cache.reserve_pending("other".to_owned()));
        assert_eq!(cache.get_cloned("pending"), Some(IconState::Pending));
        assert_eq!(cache.get_cloned("other"), None);

        cache.insert(
            "pending".to_owned(),
            IconState::Ready("ready-url".to_owned()),
        );
        assert!(cache.reserve_pending("other".to_owned()));
        assert_eq!(cache.get_cloned("pending"), None);
        assert_eq!(cache.get_cloned("other"), Some(IconState::Pending));
    }

    #[test]
    fn clears_process_bundle_cache() {
        let temp_dir = unique_temp_dir();
        let cache = IconCache {
            cache_dir: temp_dir,
            icons: Arc::new(Mutex::new(IconMemoryCache::new(MAX_IN_MEMORY_ICON_ENTRIES))),
            process_bundles: Mutex::new(HashMap::from([(
                42,
                Some(PathBuf::from("/Applications/Sample.app")),
            )])),
            render_limiter: Arc::new(RenderLimiter::new()),
            proxy: None,
        };

        cache.clear_process_bundle_cache();

        assert!(cache.process_bundles.lock().expect("cache lock").is_empty());
    }

    #[test]
    fn cleanup_removes_matching_stale_webp_files() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let stale_cache_file = temp_dir.join("abc-webp-v1-64px.webp");
        fs::write(&stale_cache_file, b"stale").expect("stale file should be written");
        set_modified(
            &stale_cache_file,
            SystemTime::now()
                .checked_sub(ICON_DISK_CACHE_TTL + Duration::from_secs(60))
                .expect("stale time should be valid"),
        );

        cleanup_stale_disk_cache_entries(&temp_dir);

        assert!(!stale_cache_file.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn cleanup_keeps_matching_recent_webp_files() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let recent_cache_file = temp_dir.join("abc-webp-v1-64px.webp");
        fs::write(&recent_cache_file, b"recent").expect("recent file should be written");

        cleanup_stale_disk_cache_entries(&temp_dir);

        assert!(recent_cache_file.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn cleanup_keeps_non_cache_webp_and_unrelated_files() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let non_cache_webp = temp_dir.join("app-icon.webp");
        let old_format_cache = temp_dir.join("abc-png-v1-64px.webp");
        let unrelated_file = temp_dir.join("notes.txt");
        for path in [&non_cache_webp, &old_format_cache, &unrelated_file] {
            fs::write(path, b"stale").expect("file should be written");
            set_modified(
                path,
                SystemTime::now()
                    .checked_sub(ICON_DISK_CACHE_TTL + Duration::from_secs(60))
                    .expect("stale time should be valid"),
            );
        }

        cleanup_stale_disk_cache_entries(&temp_dir);

        assert!(non_cache_webp.exists());
        assert!(old_format_cache.exists());
        assert!(unrelated_file.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn cleanup_keeps_directories_and_temp_render_files() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let matching_directory = temp_dir.join("abc-webp-v1-64px.webp");
        let temp_render_file = temp_dir.join("abc-webp-v1-64px.tmp.webp");
        fs::create_dir_all(&matching_directory).expect("matching directory should be created");
        fs::write(&temp_render_file, b"temp").expect("temp render file should be written");
        set_modified(
            &temp_render_file,
            SystemTime::now()
                .checked_sub(ICON_DISK_CACHE_TTL + Duration::from_secs(60))
                .expect("stale time should be valid"),
        );

        cleanup_stale_disk_cache_entries(&temp_dir);

        assert!(matching_directory.exists());
        assert!(temp_render_file.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn cleanup_ignores_missing_cache_dir() {
        let temp_dir = unique_temp_dir();

        cleanup_stale_disk_cache_entries(&temp_dir);

        assert!(!temp_dir.exists());
    }

    fn set_modified(path: &Path, modified: SystemTime) {
        File::options()
            .write(true)
            .open(path)
            .expect("file should be opened")
            .set_times(FileTimes::new().set_modified(modified))
            .expect("file modified time should be set");
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("runx-icon-test-{}-{nanos}", process::id()))
    }
}
