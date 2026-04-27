use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result};
use objc2_app_kit::NSRunningApplication;
use plist::{Dictionary, Value};

use super::ICON_CACHE_FORMAT_VERSION;

pub(super) fn process_bundle_path(pid: i64) -> Option<PathBuf> {
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

pub(super) fn find_bundle_icon_source(bundle_path: &Path) -> Result<Option<PathBuf>> {
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

pub(super) fn cache_key_for_bundle(path: &Path) -> String {
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::{bundle_root_from_executable, cache_key_for_bundle, find_bundle_icon_source};

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
