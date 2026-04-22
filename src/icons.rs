//! Icon extraction and caching for search results.
//!
//! Providers ask this module for icons so they can stay focused on search
//! semantics instead of plist parsing, process inspection, and PNG rendering.

use std::{
    borrow::Cow,
    collections::{HashMap, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, MutexGuard},
};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use directories::BaseDirs;
use image::{ExtendedColorType, ImageReader, codecs::webp::WebPEncoder};
use objc2_app_kit::NSRunningApplication;
use plist::{Dictionary, Value};
use wry::http::{
    Request, Response, StatusCode,
    header::{CACHE_CONTROL, CONTENT_TYPE},
};

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
    icons: Mutex<HashMap<String, Option<String>>>,
    process_bundles: Mutex<HashMap<i64, Option<PathBuf>>>,
}

impl IconCache {
    /// Creates the icon cache rooted in the user's cache directory.
    pub fn new() -> Result<Self> {
        let base_dirs =
            BaseDirs::new().context("could not resolve the current user's home directory")?;
        let cache_dir = base_dirs.cache_dir().join("runx/icons");
        fs::create_dir_all(&cache_dir)
            .with_context(|| format!("failed to create {}", cache_dir.display()))?;

        Ok(Self {
            cache_dir,
            icons: Mutex::new(HashMap::new()),
            process_bundles: Mutex::new(HashMap::new()),
        })
    }

    /// Resolves an icon for an application bundle path and returns a data URL.
    pub fn icon_for_bundle<P: AsRef<Path>>(&self, bundle_path: P) -> Option<String> {
        let bundle_path = bundle_path.as_ref();
        let key = format!("bundle:{}", bundle_path.display());
        if let Some(icon) = self.cached_icon(&key) {
            return icon;
        }

        let icon = self.load_bundle_icon(bundle_path).ok().flatten();
        self.store_icon(key, icon.clone());
        icon
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

    fn cached_icon(&self, key: &str) -> Option<Option<String>> {
        let cache = lock_or_recover(&self.icons);
        cache.get(key).cloned()
    }

    fn store_icon(&self, key: String, value: Option<String>) {
        let mut cache = lock_or_recover(&self.icons);
        cache.insert(key, value);
    }

    fn load_bundle_icon(&self, bundle_path: &Path) -> Result<Option<String>> {
        let Some(icon_source) = find_bundle_icon_source(bundle_path)? else {
            return Ok(None);
        };
        let icon_key = stable_hash(bundle_path);

        let webp_path = self.webp_path_for_key(&icon_key);
        if !webp_path.exists() {
            render_webp_icon(&icon_source, &webp_path)?;
        }

        Ok(Some(icon_protocol_url(&icon_key)))
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
    let info_path = bundle_path.join("Contents/Info.plist");
    if !info_path.exists() {
        return Ok(None);
    }

    let plist = Value::from_file(&info_path)
        .with_context(|| format!("failed to parse {}", info_path.display()))?;
    let Some(dict) = plist.as_dictionary() else {
        return Ok(None);
    };

    let resources_dir = bundle_path.join("Contents/Resources");
    for candidate in icon_name_candidates(dict) {
        for path in icon_file_candidates(&resources_dir, &candidate) {
            if path.exists() {
                return Ok(Some(path));
            }
        }
    }

    Ok(None)
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
        for extension in ["icns", "png", "jpg", "jpeg"] {
            candidates.push(resources_dir.join(format!("{icon_name}.{extension}")));
        }
        candidates.push(resources_dir.join(path));
    }

    candidates
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

fn stable_hash(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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
    use super::{bundle_root_from_executable, icon_key_from_request_path, render_webp_icon};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use std::{
        fs,
        path::{Path, PathBuf},
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

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
