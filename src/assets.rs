//! Asset lookup and rendering for packaged versus source-run builds.
//!
//! The tray icon can come from bundled app resources or be rendered from the
//! repository asset on demand during development.

use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use directories::BaseDirs;

const TRAY_ICON_RESOURCE: &str = "RunxStatusTemplate.png";
const TRAY_ICON_SOURCE: &str = "assets/runx-status-template.svg";
const TRAY_ICON_SIZE: u32 = 36;

/// Returns the PNG path used by the tray integration.
pub fn tray_icon_path() -> Result<PathBuf> {
    if let Some(path) = bundled_resource(TRAY_ICON_RESOURCE) {
        return Ok(path);
    }

    let source = manifest_asset(TRAY_ICON_SOURCE);
    let base_dirs =
        BaseDirs::new().context("could not resolve the current user's home directory")?;
    let cache_dir = base_dirs.cache_dir().join("runx/assets");
    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create {}", cache_dir.display()))?;
    let fingerprint = asset_fingerprint(&source)?;
    let output = cache_dir.join(format!(
        "runx-status-template-{fingerprint:016x}-{}px.png",
        TRAY_ICON_SIZE
    ));
    if !output.exists() {
        render_svg_to_png(&source, &output, TRAY_ICON_SIZE)?;
    }
    Ok(output)
}

fn bundled_resource(relative: &str) -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let contents_dir = executable.parent()?.parent()?;
    let resource_path = contents_dir.join("Resources").join(relative);
    resource_path.exists().then_some(resource_path)
}

fn manifest_asset(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn asset_fingerprint(path: &Path) -> Result<u64> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

fn render_svg_to_png(source: &Path, output: &Path, size: u32) -> Result<()> {
    let size = size.to_string();
    let output_result = Command::new("sips")
        .args([
            "-Z",
            &size,
            "-s",
            "format",
            "png",
            &source.to_string_lossy(),
            "--out",
            &output.to_string_lossy(),
        ])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run `sips` for {}", source.display()))?;

    if output_result.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output_result.stderr)
        .trim()
        .to_owned();
    if stderr.is_empty() {
        bail!("`sips` exited with status {}", output_result.status);
    }
    bail!("{stderr}");
}
