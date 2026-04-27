use std::{env, fs, path::Path};

use anyhow::{Context, Result, bail};
use resvg::{
    tiny_skia::{Pixmap, Transform},
    usvg,
};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let input = args.next().context("missing input SVG path")?;
    let output = args.next().context("missing output PNG path")?;
    let size = args
        .next()
        .context("missing output size")?
        .parse::<u32>()
        .context("output size must be a positive integer")?;
    if size == 0 {
        bail!("output size must be greater than zero");
    }
    if args.next().is_some() {
        bail!("usage: svg2png <input.svg> <output.png> <size>");
    }

    render(Path::new(&input), Path::new(&output), size)
}

fn render(input: &Path, output: &Path, size: u32) -> Result<()> {
    let mut options = usvg::Options {
        resources_dir: fs::canonicalize(input)
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf)),
        ..usvg::Options::default()
    };
    options.fontdb_mut().load_system_fonts();

    let svg = fs::read(input).with_context(|| format!("failed to read {}", input.display()))?;
    let tree = usvg::Tree::from_data(&svg, &options)
        .with_context(|| format!("failed to parse {}", input.display()))?;
    let svg_size = tree.size();
    let source_width = svg_size.width();
    let source_height = svg_size.height();
    if source_width <= 0.0 || source_height <= 0.0 {
        bail!("SVG has an empty viewport: {}", input.display());
    }

    let target_size = size as f32;
    let scale = target_size / source_width.max(source_height);
    let offset_x = (target_size - source_width * scale) / 2.0;
    let offset_y = (target_size - source_height * scale) / 2.0;
    let transform = Transform::from_row(scale, 0.0, 0.0, scale, offset_x, offset_y);

    let mut pixmap =
        Pixmap::new(size, size).with_context(|| format!("failed to allocate {size}x{size} PNG"))?;
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap
        .save_png(output)
        .with_context(|| format!("failed to write {}", output.display()))
}
