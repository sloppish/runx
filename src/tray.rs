use std::{fs::File, io::BufReader, path::Path};

use anyhow::{Context, Result, bail};
use png::{ColorType, Decoder, Transformations};
use tao::event_loop::EventLoopProxy;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

use crate::{assets::tray_icon_path, types::AppEvent};

const TRAY_ID: &str = "runx-tray";
const MENU_OPEN_ID: &str = "tray-open";
const MENU_QUIT_ID: &str = "tray-quit";

pub struct TrayState {
    _tray: TrayIcon,
    _menu: Menu,
    _open_item: MenuItem,
    _quit_item: MenuItem,
}

impl TrayState {
    pub fn install(proxy: EventLoopProxy<AppEvent>) -> Result<Self> {
        let menu = Menu::new();
        let open_item = MenuItem::with_id(MENU_OPEN_ID, "Open Runx", true, None);
        let quit_item = MenuItem::with_id(MENU_QUIT_ID, "Quit Runx", true, None);
        menu.append_items(&[&open_item, &quit_item])
            .context("failed to build the tray menu")?;

        let open_id = open_item.id().clone();
        let quit_id = quit_item.id().clone();
        MenuEvent::set_event_handler(Some({
            let proxy = proxy.clone();
            move |event: MenuEvent| {
                let app_event = if event.id == open_id {
                    Some(AppEvent::TrayOpen)
                } else if event.id == quit_id {
                    Some(AppEvent::Quit)
                } else {
                    None
                };
                if let Some(event) = app_event {
                    let _ = proxy.send_event(event);
                }
            }
        }));

        TrayIconEvent::set_event_handler(Some({
            let proxy = proxy.clone();
            move |event| {
                if matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                ) {
                    let _ = proxy.send_event(AppEvent::TrayToggle);
                }
            }
        }));

        let icon_path = tray_icon_path()?;
        let icon = load_icon(&icon_path)?;
        let tray = TrayIconBuilder::new()
            .with_id(TRAY_ID)
            .with_icon(icon)
            .with_icon_as_template(true)
            .with_tooltip("Runx")
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(false)
            .build()
            .context("failed to create the tray icon")?;

        Ok(Self {
            _tray: tray,
            _menu: menu,
            _open_item: open_item,
            _quit_item: quit_item,
        })
    }
}

fn load_icon(path: &Path) -> Result<Icon> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut decoder = Decoder::new(BufReader::new(file));
    decoder.set_transformations(Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .with_context(|| format!("failed to decode {}", path.display()))?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buffer)
        .with_context(|| format!("failed to read icon data from {}", path.display()))?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = rgba_pixels(pixels, info.color_type)?;
    Icon::from_rgba(rgba, info.width, info.height)
        .with_context(|| format!("failed to build tray icon from {}", path.display()))
}

fn rgba_pixels(pixels: &[u8], color_type: ColorType) -> Result<Vec<u8>> {
    let rgba = match color_type {
        ColorType::Rgba => pixels.to_vec(),
        ColorType::Rgb => pixels
            .chunks_exact(3)
            .flat_map(|chunk| [chunk[0], chunk[1], chunk[2], 255])
            .collect(),
        ColorType::Grayscale => pixels
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        ColorType::GrayscaleAlpha => pixels
            .chunks_exact(2)
            .flat_map(|chunk| [chunk[0], chunk[0], chunk[0], chunk[1]])
            .collect(),
        ColorType::Indexed => bail!("indexed PNG tray icons are not supported"),
    };
    Ok(rgba)
}
