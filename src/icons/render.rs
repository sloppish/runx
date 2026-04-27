use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result};
use image::{ExtendedColorType, ImageReader, codecs::webp::WebPEncoder};
use objc2::AnyThread;
use objc2_app_kit::{
    NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace, NSGraphicsContext, NSImage,
    NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::{ICON_RENDER_SIZE, IconLookupMode, bundle::find_bundle_icon_source};

pub(super) fn render_bundle_icon_to_webp(
    bundle_path: &Path,
    webp_path: &Path,
    mode: IconLookupMode,
) -> Result<()> {
    let source_error = match find_bundle_icon_source(bundle_path)? {
        Some(icon_source)
            if mode == IconLookupMode::Full && should_render_with_workspace_icon(&icon_source) =>
        {
            match render_workspace_icon_to_webp(bundle_path, webp_path) {
                Ok(()) => return Ok(()),
                Err(error) => Some(error),
            }
        }
        Some(icon_source) => match render_webp_icon(&icon_source, webp_path) {
            Ok(()) => return Ok(()),
            Err(error) => Some(error),
        },
        None => None,
    };

    if mode == IconLookupMode::DirectResourceOnly {
        anyhow::bail!("no direct icon resource found in {}", bundle_path.display());
    }

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

fn should_render_with_workspace_icon(icon_source: &Path) -> bool {
    icon_source.extension().and_then(|value| value.to_str()) == Some("icns")
        && icon_source
            .parent()
            .is_some_and(|parent| parent.join("Assets.car").exists())
}

fn render_workspace_icon_to_webp(bundle_path: &Path, webp_path: &Path) -> Result<()> {
    let bundle_path_text = bundle_path.to_string_lossy();
    let bundle_path = NSString::from_str(&bundle_path_text);
    let workspace = NSWorkspace::sharedWorkspace();
    let icon = workspace.iconForFile(&bundle_path);
    render_nsimage_to_webp(&icon, webp_path)
}

fn render_nsimage_to_webp(icon: &NSImage, webp_path: &Path) -> Result<()> {
    let temp_tiff_path = webp_path.with_extension("tmp.tiff");
    let size = f64::from(ICON_RENDER_SIZE);
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            ICON_RENDER_SIZE as isize,
            ICON_RENDER_SIZE as isize,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }
    .context("failed to create bitmap image representation")?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)
        .context("failed to create bitmap graphics context")?;
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size, size));

    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    icon.setSize(NSSize::new(size, size));
    icon.drawInRect_fromRect_operation_fraction(
        rect,
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
        NSCompositingOperation::Copy,
        1.0,
    );
    NSGraphicsContext::restoreGraphicsState_class();

    let tiff = bitmap
        .TIFFRepresentation()
        .context("macOS failed to rasterize icon into a TIFF representation")?;
    fs::write(&temp_tiff_path, tiff.to_vec())
        .with_context(|| format!("failed to create {}", temp_tiff_path.display()))?;

    let render_result = render_webp_icon(&temp_tiff_path, webp_path);
    let _ = fs::remove_file(&temp_tiff_path);
    render_result
}

pub(super) fn render_webp_icon(icon_source: &Path, webp_path: &Path) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        process,
        time::{SystemTime, UNIX_EPOCH},
    };

    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::render_webp_icon;

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
