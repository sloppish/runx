use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use directories::BaseDirs;
use plist::{Dictionary, Value};

const SYSTEM_SETTINGS_APP: &str = "/System/Applications/System Settings.app";
const ICON_RENDER_SIZE: u32 = 128;

pub struct IconCache {
    cache_dir: PathBuf,
    icons: Mutex<HashMap<String, Option<String>>>,
    process_bundles: Mutex<HashMap<i64, Option<PathBuf>>>,
}

impl IconCache {
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

    pub fn icon_for_pid(&self, pid: i64) -> Option<String> {
        let bundle_path = {
            let cache = self
                .process_bundles
                .lock()
                .expect("process icon cache poisoned");
            cache.get(&pid).cloned()
        };

        let bundle_path = match bundle_path {
            Some(path) => path,
            None => {
                let resolved = process_bundle_path(pid);
                let mut cache = self
                    .process_bundles
                    .lock()
                    .expect("process icon cache poisoned");
                cache.insert(pid, resolved.clone());
                resolved
            }
        }?;

        self.icon_for_bundle(bundle_path)
    }

    pub fn system_settings_icon(&self) -> Option<String> {
        self.icon_for_bundle(SYSTEM_SETTINGS_APP)
    }

    fn cached_icon(&self, key: &str) -> Option<Option<String>> {
        let cache = self.icons.lock().expect("icon cache poisoned");
        cache.get(key).cloned()
    }

    fn store_icon(&self, key: String, value: Option<String>) {
        let mut cache = self.icons.lock().expect("icon cache poisoned");
        cache.insert(key, value);
    }

    fn load_bundle_icon(&self, bundle_path: &Path) -> Result<Option<String>> {
        let Some(icon_source) = find_bundle_icon_source(bundle_path)? else {
            return Ok(None);
        };

        let png_path = self.cache_dir.join(format!(
            "{}-{}px.png",
            stable_hash(bundle_path),
            ICON_RENDER_SIZE
        ));
        if !png_path.exists() {
            render_png_icon(&icon_source, &png_path)?;
        }

        let bytes = fs::read(&png_path)
            .with_context(|| format!("failed to read {}", png_path.display()))?;
        Ok(Some(format!(
            "data:image/png;base64,{}",
            STANDARD.encode(bytes)
        )))
    }
}

fn process_bundle_path(pid: i64) -> Option<PathBuf> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .stdin(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let command_path = String::from_utf8(output.stdout).ok()?;
    let command_path = PathBuf::from(command_path.trim());
    bundle_root_from_executable(&command_path)
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

fn render_png_icon(icon_source: &Path, png_path: &Path) -> Result<()> {
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
            &png_path.to_string_lossy(),
        ])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run `sips` for {}", icon_source.display()))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        anyhow::bail!("`sips` exited with status {}", output.status);
    }
    anyhow::bail!("{stderr}");
}

fn stable_hash(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::bundle_root_from_executable;
    use std::path::Path;

    #[test]
    fn finds_app_bundle_from_executable_path() {
        let path = Path::new("/Applications/Alacritty.app/Contents/MacOS/alacritty");
        assert_eq!(
            bundle_root_from_executable(path).as_deref(),
            Some(Path::new("/Applications/Alacritty.app"))
        );
    }

    #[test]
    fn returns_none_without_bundle_ancestor() {
        let path = Path::new("/usr/bin/ssh");
        assert!(bundle_root_from_executable(path).is_none());
    }
}
