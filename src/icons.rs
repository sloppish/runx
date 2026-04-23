//! Icon extraction and caching for search results.
//!
//! Providers ask this module for icons so they can stay focused on search
//! semantics instead of plist parsing, process inspection, and PNG rendering.

use std::{
    borrow::Cow,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use directories::BaseDirs;
use image::{ExtendedColorType, ImageReader, codecs::webp::WebPEncoder};
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_foundation::NSString;
use plist::{Dictionary, Value};
use tao::event_loop::EventLoopProxy;
use wry::http::{
    Request, Response, StatusCode,
    header::{CACHE_CONTROL, CONTENT_TYPE},
};

use crate::{debug_log, types::AppEvent};

const SYSTEM_SETTINGS_APP_CANDIDATES: [&str; 2] = [
    "/System/Applications/System Settings.app",
    "/System/Applications/System Preferences.app",
];
const ICON_PROTOCOL_SCHEME: &str = "runx";
const ICON_PROTOCOL_HOST: &str = "localhost";
const ICON_CACHE_FORMAT_VERSION: &str = "webp-v1";
const ICON_RENDER_SIZE: u32 = 64;

/// In-memory and on-disk cache for bundle and process icons.
pub struct IconCache {
    cache_dir: PathBuf,
    icons: Arc<Mutex<HashMap<String, IconState>>>,
    process_bundles: Mutex<HashMap<i64, Option<PathBuf>>>,
    proxy: Option<EventLoopProxy<AppEvent>>,
}

#[derive(Clone, Debug)]
enum IconState {
    Ready(String),
    Pending,
    Missing,
}

impl IconCache {
    /// Creates the icon cache rooted in the user's cache directory.
    pub fn new(proxy: Option<EventLoopProxy<AppEvent>>) -> Result<Self> {
        let base_dirs =
            BaseDirs::new().context("could not resolve the current user's home directory")?;
        let cache_dir = base_dirs.cache_dir().join("runx/icons");
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("failed to create {}", cache_dir.display()))?;

        Ok(Self {
            cache_dir,
            icons: Arc::new(Mutex::new(HashMap::new())),
            process_bundles: Mutex::new(HashMap::new()),
            proxy,
        })
    }

    /// Resolves an icon for an application bundle path and returns a protocol URL when ready.
    pub fn icon_for_bundle<P: AsRef<Path>>(&self, bundle_path: P) -> Option<String> {
        let bundle_path = bundle_path.as_ref();
        let key = format!("bundle:{}", bundle_path.display());

        {
            let mut cache = lock_or_recover(&self.icons);
            match cache.get(&key).cloned() {
                Some(IconState::Ready(url)) => return Some(url),
                Some(IconState::Pending | IconState::Missing) => return None,
                None => {
                    cache.insert(key.clone(), IconState::Pending);
                }
            }
        }

        self.resolve_or_schedule_bundle_icon(key, bundle_path)
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

    fn resolve_or_schedule_bundle_icon(&self, key: String, bundle_path: &Path) -> Option<String> {
        if !bundle_path.exists() {
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

        self.spawn_bundle_icon_render(key, bundle_path.to_path_buf(), webp_path, url);
        None
    }

    fn spawn_bundle_icon_render(
        &self,
        key: String,
        bundle_path: PathBuf,
        webp_path: PathBuf,
        url: String,
    ) {
        let icons = Arc::clone(&self.icons);
        let proxy = self.proxy.clone();
        let temp_webp_path = webp_path.with_extension("tmp.webp");
        let thread_key = key.clone();

        let spawn_result = thread::Builder::new()
            .name("runx-icon-render".to_owned())
            .spawn(move || {
                let render_result = render_bundle_icon_to_webp(&bundle_path, &temp_webp_path)
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
                    debug_log::append(format!(
                        "icon render failed bundle={} error={error:#}",
                        bundle_path.display()
                    ));
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

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

fn process_bundle_path(pid: i64) -> Option<PathBuf> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as _)?;
    if let Some(bundle_url) = app.bundleURL()
        && let Some(path) = bundle_url.path()
    {
        return Some(PathBuf::from(path.to_string()));
    }

    let executable_url = app.executableURL()?;
    let executable_path = executable_url.path()?;
    let executable_path = PathBuf::from(executable_path.to_string());
    bundle_root_from_executable(&executable_path)
}

fn bundle_root_from_executable(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| {
            matches!(
                ancestor.extension().and_then(|value| value.to_str()),
                Some("app" | "appex" | "prefPane")
            )
        })
        .map(Path::to_path_buf)
}

fn find_bundle_icon_source(bundle_path: &Path) -> Result<Option<PathBuf>> {
    for layout in bundle_icon_layouts(bundle_path) {
        let plist = Value::from_file(&layout.info_path)
            .with_context(|| format!("failed to parse {}", layout.info_path.display()))?;
        let Some(dict) = plist.as_dictionary() else {
            continue;
        };

        for candidate in icon_name_candidates(dict) {
            for path in icon_file_candidates(&layout.resources_dir, &candidate) {
                if path.exists() {
                    return Ok(Some(path));
                }
            }
        }
    }

    Ok(None)
}

struct IconBundleLayout {
    info_path: PathBuf,
    resources_dir: PathBuf,
}

fn bundle_icon_layouts(bundle_path: &Path) -> Vec<IconBundleLayout> {
    let mut layouts = Vec::new();
    let standard_info_path = bundle_path.join("Contents/Info.plist");
    if standard_info_path.exists() {
        layouts.push(IconBundleLayout {
            info_path: standard_info_path,
            resources_dir: bundle_path.join("Contents/Resources"),
        });
    }

    let wrapper_dir = bundle_path.join("Wrapper");
    let Ok(entries) = fs::read_dir(wrapper_dir) else {
        return layouts;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("app") {
            continue;
        }

        let info_path = path.join("Info.plist");
        if info_path.exists() {
            layouts.push(IconBundleLayout {
                info_path,
                resources_dir: path,
            });
        }
    }

    layouts
}

fn icon_name_candidates(dict: &Dictionary) -> Vec<String> {
    let mut candidates = Vec::new();

    if let Some(name) = dict.get("CFBundleIconFile").and_then(Value::as_string) {
        candidates.push(name.to_owned());
    }

    if let Some(name) = dict.get("CFBundleIconName").and_then(Value::as_string) {
        candidates.push(name.to_owned());
    }

    if let Some(primary) = dict
        .get("CFBundleIcons")
        .and_then(Value::as_dictionary)
        .and_then(|icons| icons.get("CFBundlePrimaryIcon"))
        .and_then(Value::as_dictionary)
        && let Some(icon_files) = primary.get("CFBundleIconFiles").and_then(Value::as_array)
    {
        for value in icon_files.iter().rev() {
            if let Some(name) = value.as_string() {
                candidates.push(name.to_owned());
            }
        }
    }

    if let Some(icon_files) = dict.get("CFBundleIconFiles").and_then(Value::as_array) {
        for value in icon_files.iter().rev() {
            if let Some(name) = value.as_string() {
                candidates.push(name.to_owned());
            }
        }
    }

    candidates
}

fn icon_file_candidates(resources_dir: &Path, icon_name: &str) -> Vec<PathBuf> {
    let path = Path::new(icon_name);
    let has_extension = path.extension().is_some();
    let mut candidates = Vec::new();

    if has_extension {
        candidates.push(resources_dir.join(path));
    } else {
        for suffix in [
            ".icns",
            "@3x.png",
            "@2x.png",
            "@3x~ipad.png",
            "@2x~ipad.png",
            ".png",
            ".jpg",
            ".jpeg",
        ] {
            candidates.push(resources_dir.join(format!("{icon_name}{suffix}")));
        }
        candidates.push(resources_dir.join(path));
    }

    candidates
}

fn render_bundle_icon_to_webp(bundle_path: &Path, webp_path: &Path) -> Result<()> {
    let source_error = match find_bundle_icon_source(bundle_path)? {
        Some(icon_source) => match render_webp_icon(&icon_source, webp_path) {
            Ok(()) => return Ok(()),
            Err(error) => Some(error),
        },
        None => None,
    };

    match render_workspace_icon_to_webp(bundle_path, webp_path) {
        Ok(()) => Ok(()),
        Err(error) => {
            if let Some(source_error) = source_error {
                Err(error.context(format!(
                    "NSWorkspace fallback failed after plist icon render failed: {source_error:#}"
                )))
            } else {
                Err(error)
            }
        }
    }
}

fn render_workspace_icon_to_webp(bundle_path: &Path, webp_path: &Path) -> Result<()> {
    let temp_tiff_path = webp_path.with_extension("tmp.tiff");
    let bundle_path_text = bundle_path.to_string_lossy();
    let bundle_path = NSString::from_str(&bundle_path_text);
    let workspace = NSWorkspace::sharedWorkspace();
    let icon = workspace.iconForFile(&bundle_path);
    let tiff = icon
        .TIFFRepresentation()
        .context("macOS returned an icon without a TIFF representation")?;
    fs::write(&temp_tiff_path, tiff.to_vec())
        .with_context(|| format!("failed to create {}", temp_tiff_path.display()))?;

    let render_result = render_webp_icon(&temp_tiff_path, webp_path);
    let _ = fs::remove_file(&temp_tiff_path);
    render_result
}

fn render_webp_icon(icon_source: &Path, webp_path: &Path) -> Result<()> {
    let temp_png_path = webp_path.with_extension("tmp.png");
    let size = ICON_RENDER_SIZE.to_string();
    let output = Command::new("sips")
        .args([
            "-Z",
            &size,
            "-s",
            "format",
            "png",
            &icon_source.to_string_lossy(),
            "--out",
            &temp_png_path.to_string_lossy(),
        ])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run `sips` for {}", icon_source.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.is_empty() {
            anyhow::bail!("`sips` exited with status {}", output.status);
        }
        anyhow::bail!("{stderr}");
    }

    let encode_result = (|| -> Result<()> {
        let image = ImageReader::open(&temp_png_path)
            .with_context(|| format!("failed to open {}", temp_png_path.display()))?
            .decode()
            .with_context(|| format!("failed to decode {}", temp_png_path.display()))?
            .into_rgba8();
        let (width, height) = image.dimensions();
        let mut output = fs::File::create(webp_path)
            .with_context(|| format!("failed to create {}", webp_path.display()))?;
        WebPEncoder::new_lossless(&mut output)
            .encode(image.as_raw(), width, height, ExtendedColorType::Rgba8)
            .with_context(|| format!("failed to encode {}", webp_path.display()))?;
        Ok(())
    })();

    let _ = fs::remove_file(&temp_png_path);
    encode_result
}

fn icon_protocol_url(icon_key: &str) -> String {
    format!("{ICON_PROTOCOL_SCHEME}://{ICON_PROTOCOL_HOST}/icon/{icon_key}.webp")
}

fn icon_key_from_request_path(path: &str) -> Option<&str> {
    let key = path.strip_prefix("/icon/")?.strip_suffix(".webp")?;
    key.chars().all(|ch| ch.is_ascii_hexdigit()).then_some(key)
}

fn response_with_status(
    status: StatusCode,
    content_type: &'static str,
    body: &'static [u8],
) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .body(Cow::Borrowed(body))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(body)))
}

fn cache_key_for_bundle(path: &Path) -> String {
    let signature = bundle_metadata_signature(path);
    let mut hasher = StableCacheHasher::new();
    hasher.update_str(ICON_CACHE_FORMAT_VERSION);
    hasher.update_str(&path.to_string_lossy());
    hasher.update_optional_u128(signature.bundle_modified_at);
    hasher.update_optional_str(signature.bundle_version.as_deref());
    format!("{:016x}", hasher.finish())
}

fn bundle_metadata_signature(path: &Path) -> BundleMetadataSignature {
    BundleMetadataSignature {
        bundle_modified_at: modified_at(path),
        bundle_version: bundle_version(path),
    }
}

struct BundleMetadataSignature {
    bundle_modified_at: Option<u128>,
    bundle_version: Option<String>,
}

struct StableCacheHasher {
    state: u64,
}

impl StableCacheHasher {
    fn new() -> Self {
        Self {
            state: 0xcbf2_9ce4_8422_2325,
        }
    }

    fn update_str(&mut self, value: &str) {
        self.update_field(value.as_bytes());
    }

    fn update_optional_str(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                self.update(&[1]);
                self.update_str(value);
            }
            None => self.update(&[0]),
        }
    }

    fn update_optional_u128(&mut self, value: Option<u128>) {
        match value {
            Some(value) => {
                self.update(&[1]);
                self.update_field(&value.to_le_bytes());
            }
            None => self.update(&[0]),
        }
    }

    fn update_field(&mut self, bytes: &[u8]) {
        self.update(&bytes.len().to_le_bytes());
        self.update(bytes);
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(self) -> u64 {
        self.state
    }
}

fn modified_at(path: &Path) -> Option<u128> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

fn bundle_version(path: &Path) -> Option<String> {
    for layout in bundle_icon_layouts(path) {
        let Ok(plist) = Value::from_file(layout.info_path) else {
            continue;
        };
        let Some(dict) = plist.as_dictionary() else {
            continue;
        };

        let identifier = dict
            .get("CFBundleIdentifier")
            .and_then(Value::as_string)
            .unwrap_or_default();
        let version = dict
            .get("CFBundleVersion")
            .or_else(|| dict.get("CFBundleShortVersionString"))
            .and_then(Value::as_string)
            .unwrap_or_default();

        if !identifier.is_empty() || !version.is_empty() {
            return Some(format!("{identifier}:{version}"));
        }
    }

    None
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
    use super::{
        IconCache, bundle_root_from_executable, cache_key_for_bundle, find_bundle_icon_source,
        icon_key_from_request_path, icon_protocol_url, render_webp_icon,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
        process,
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };
    use wry::http::{Request, StatusCode};

    #[test]
    fn finds_app_bundle_from_executable_path() {
        let path = Path::new("/Applications/SampleApp.app/Contents/MacOS/sample-app");
        assert_eq!(
            bundle_root_from_executable(path).as_deref(),
            Some(Path::new("/Applications/SampleApp.app"))
        );
    }

    #[test]
    fn returns_none_without_bundle_ancestor() {
        let path = Path::new("/usr/bin/ssh");
        assert!(bundle_root_from_executable(path).is_none());
    }

    #[test]
    fn extracts_icon_key_from_protocol_path() {
        assert_eq!(
            icon_key_from_request_path("/icon/deadbeef00cafe42.webp"),
            Some("deadbeef00cafe42")
        );
    }

    #[test]
    fn rejects_non_icon_protocol_paths() {
        assert!(icon_key_from_request_path("/icons/deadbeef.webp").is_none());
        assert!(icon_key_from_request_path("/icon/not-hex.webp").is_none());
        assert!(icon_key_from_request_path("/icon/deadbeef.svg").is_none());
    }

    #[test]
    fn finds_icons_in_wrapped_app_bundles() {
        let temp_dir = unique_temp_dir();
        let bundle_path = temp_dir.join("Outer.app");
        let wrapped_path = bundle_path.join("Wrapper/Inner.app");
        fs::create_dir_all(&wrapped_path).expect("wrapped app should be created");
        let icon_path = wrapped_path.join("AppIcon60x60@2x.png");
        fs::write(&icon_path, tiny_png_bytes()).expect("wrapped icon should be written");
        fs::write(
            wrapped_path.join("Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>com.example.wrapped</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>CFBundleIcons</key>
  <dict>
    <key>CFBundlePrimaryIcon</key>
    <dict>
      <key>CFBundleIconFiles</key>
      <array>
        <string>AppIcon60x60</string>
      </array>
    </dict>
  </dict>
</dict>
</plist>
"#,
        )
        .expect("wrapped plist should be written");

        assert_eq!(
            find_bundle_icon_source(&bundle_path).expect("icon lookup should succeed"),
            Some(icon_path.clone())
        );
        assert_ne!(
            cache_key_for_bundle(&bundle_path),
            cache_key_for_bundle(&temp_dir)
        );

        let _ = fs::remove_file(&icon_path);
        let _ = fs::remove_file(wrapped_path.join("Info.plist"));
        let _ = fs::remove_dir(&wrapped_path);
        let _ = fs::remove_dir(bundle_path.join("Wrapper"));
        let _ = fs::remove_dir(&bundle_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn returns_ready_disk_cache_entry_without_rendering() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let bundle_path = temp_dir.join("Sample.app");
        fs::create_dir_all(&bundle_path).expect("bundle dir should be created");

        let cache = IconCache {
            cache_dir: temp_dir.clone(),
            icons: Arc::new(Mutex::new(HashMap::new())),
            process_bundles: Mutex::new(HashMap::new()),
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
    fn renders_webp_cache_entries() {
        let temp_dir = unique_temp_dir();
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");

        let source_path = temp_dir.join("source.png");
        let output_path = temp_dir.join("icon.webp");
        fs::write(&source_path, tiny_png_bytes()).expect("source png should be written");

        render_webp_icon(&source_path, &output_path).expect("webp render should succeed");

        let bytes = fs::read(&output_path).expect("webp output should exist");
        assert!(bytes.starts_with(b"RIFF"));
        assert_eq!(&bytes[8..12], b"WEBP");

        let _ = fs::remove_file(&source_path);
        let _ = fs::remove_file(&output_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("runx-icon-test-{}-{nanos}", process::id()))
    }

    fn tiny_png_bytes() -> Vec<u8> {
        STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4////fwAJ+wP9KobjigAAAABJRU5ErkJggg==")
            .expect("embedded png should decode")
    }
}
