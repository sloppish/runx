//! Embeds the static frontend templates into a themed HTML document.

use serde::Serialize;

use crate::config::{UI_COLOR_TOKEN_NAMES, UiColorOverridesConfig, UiConfig};

const HTML_TEMPLATE: &str = include_str!("../ui/index.html");
const STYLE_TEMPLATE: &str = include_str!("../ui/styles.css");
const SCRIPT_SOURCE: &str = include_str!("../ui/app.js");

#[derive(Clone, Serialize)]
struct ResolvedUiColors {
    accent: String,
    panel: String,
    text: String,
    muted: String,
    canvas_bg: String,
    canvas_shadow: String,
    canvas_border: String,
    label_strong: String,
    input_bg: String,
    input_border: String,
    input_shadow: String,
    placeholder: String,
    scrollbar: String,
    item_bg: String,
    item_hover: String,
    item_selected_bg: String,
    item_selected_shadow: String,
    badge_bg: String,
    badge_border: String,
    badge_text: String,
    badge_icon_bg: String,
    chip_text: String,
    chip_bg: String,
    chip_border: String,
    config_error_bg: String,
    config_error_border: String,
    config_error_shadow: String,
    config_error_title: String,
    config_error_copy: String,
    canvas_hidden_input_bg: String,
    canvas_hidden_input_border: String,
    canvas_hidden_input_shadow: String,
    canvas_hidden_item_bg: String,
    canvas_hidden_item_hover: String,
    canvas_hidden_item_selected_bg: String,
    canvas_hidden_config_error_bg: String,
}

impl ResolvedUiColors {
    fn builtin_light() -> Self {
        Self {
            accent: "#b57614".to_owned(),
            panel: "#f2e5bc".to_owned(),
            text: "#3c3836".to_owned(),
            muted: "#7c6f64".to_owned(),
            canvas_bg: "linear-gradient(180deg, #fbf1c7, #f2e5bc)".to_owned(),
            canvas_shadow: "inset 0 1px 0 rgba(255, 255, 255, 0.34)".to_owned(),
            canvas_border: "rgba(124, 111, 100, 0.18)".to_owned(),
            label_strong: "#282828".to_owned(),
            input_bg:
                "linear-gradient(180deg, rgba(242, 229, 188, 0.98), rgba(235, 219, 178, 0.98))"
                    .to_owned(),
            input_border: "rgba(124, 111, 100, 0.18)".to_owned(),
            input_shadow: "inset 0 1px 0 rgba(251, 241, 199, 0.45)".to_owned(),
            placeholder: "#928374".to_owned(),
            scrollbar: "rgba(124, 111, 100, 0.28)".to_owned(),
            item_bg: "rgba(60, 56, 54, 0.045)".to_owned(),
            item_hover: "rgba(60, 56, 54, 0.085)".to_owned(),
            item_selected_bg:
                "linear-gradient(135deg, rgba(181, 118, 20, 0.2), rgba(235, 219, 178, 0.98))"
                    .to_owned(),
            item_selected_shadow:
                "0 18px 44px rgba(80, 73, 69, 0.12), inset 0 0 0 1px rgba(181, 118, 20, 0.24)"
                    .to_owned(),
            badge_bg:
                "linear-gradient(180deg, rgba(181, 118, 20, 0.16), rgba(251, 241, 199, 0.82))"
                    .to_owned(),
            badge_border: "rgba(124, 111, 100, 0.18)".to_owned(),
            badge_text: "#3c3836".to_owned(),
            badge_icon_bg: "rgba(60, 56, 54, 0.82)".to_owned(),
            chip_text: "#504945".to_owned(),
            chip_bg: "rgba(124, 111, 100, 0.09)".to_owned(),
            chip_border: "rgba(124, 111, 100, 0.14)".to_owned(),
            config_error_bg:
                "linear-gradient(180deg, rgba(157, 0, 6, 0.12), rgba(251, 241, 199, 0.4))"
                    .to_owned(),
            config_error_border: "rgba(157, 0, 6, 0.24)".to_owned(),
            config_error_shadow:
                "0 18px 44px rgba(80, 73, 69, 0.12), inset 0 1px 0 rgba(251, 241, 199, 0.28)"
                    .to_owned(),
            config_error_title: "#9d0006".to_owned(),
            config_error_copy: "#3c3836".to_owned(),
            canvas_hidden_input_bg: "linear-gradient(180deg, #f2e5bc, #ebdbb2)".to_owned(),
            canvas_hidden_input_border: "rgba(124, 111, 100, 0.18)".to_owned(),
            canvas_hidden_input_shadow:
                "0 10px 28px rgba(80, 73, 69, 0.12), inset 0 1px 0 rgba(251, 241, 199, 0.36)"
                    .to_owned(),
            canvas_hidden_item_bg: "rgba(242, 229, 188, 0.94)".to_owned(),
            canvas_hidden_item_hover: "rgba(235, 219, 178, 0.98)".to_owned(),
            canvas_hidden_item_selected_bg:
                "linear-gradient(135deg, rgba(181, 118, 20, 0.18), rgba(213, 196, 161, 0.98))"
                    .to_owned(),
            canvas_hidden_config_error_bg:
                "linear-gradient(180deg, rgba(157, 0, 6, 0.1), rgba(242, 229, 188, 0.98))"
                    .to_owned(),
        }
    }

    fn builtin_dark() -> Self {
        Self {
            accent: "#d79921".to_owned(),
            panel: "#282828".to_owned(),
            text: "#ebdbb2".to_owned(),
            muted: "#a89984".to_owned(),
            canvas_bg: "linear-gradient(180deg, #1d2021, #282828)".to_owned(),
            canvas_shadow: "inset 0 1px 0 rgba(251, 241, 199, 0.04)".to_owned(),
            canvas_border: "rgba(168, 153, 132, 0.18)".to_owned(),
            label_strong: "#fbf1c7".to_owned(),
            input_bg: "linear-gradient(180deg, rgba(50, 48, 47, 0.98), rgba(40, 40, 40, 0.98))"
                .to_owned(),
            input_border: "rgba(168, 153, 132, 0.16)".to_owned(),
            input_shadow: "inset 0 1px 0 rgba(251, 241, 199, 0.03)".to_owned(),
            placeholder: "#928374".to_owned(),
            scrollbar: "rgba(168, 153, 132, 0.28)".to_owned(),
            item_bg: "rgba(235, 219, 178, 0.045)".to_owned(),
            item_hover: "rgba(235, 219, 178, 0.085)".to_owned(),
            item_selected_bg:
                "linear-gradient(135deg, rgba(215, 153, 33, 0.24), rgba(60, 56, 54, 0.98))"
                    .to_owned(),
            item_selected_shadow:
                "0 18px 44px rgba(0, 0, 0, 0.24), inset 0 0 0 1px rgba(215, 153, 33, 0.24)"
                    .to_owned(),
            badge_bg: "linear-gradient(180deg, rgba(215, 153, 33, 0.18), rgba(255,255,255,0.02))"
                .to_owned(),
            badge_border: "rgba(168, 153, 132, 0.18)".to_owned(),
            badge_text: "#ebdbb2".to_owned(),
            badge_icon_bg: "rgba(29, 32, 33, 0.88)".to_owned(),
            chip_text: "#d5c4a1".to_owned(),
            chip_bg: "rgba(168, 153, 132, 0.08)".to_owned(),
            chip_border: "rgba(168, 153, 132, 0.14)".to_owned(),
            config_error_bg:
                "linear-gradient(180deg, rgba(204, 36, 29, 0.16), rgba(29, 32, 33, 0.28))"
                    .to_owned(),
            config_error_border: "rgba(204, 36, 29, 0.28)".to_owned(),
            config_error_shadow:
                "0 18px 44px rgba(0, 0, 0, 0.26), inset 0 1px 0 rgba(251, 241, 199, 0.03)"
                    .to_owned(),
            config_error_title: "#fb4934".to_owned(),
            config_error_copy: "#ebdbb2".to_owned(),
            canvas_hidden_input_bg: "linear-gradient(180deg, #32302f, #282828)".to_owned(),
            canvas_hidden_input_border: "rgba(168, 153, 132, 0.16)".to_owned(),
            canvas_hidden_input_shadow:
                "0 10px 28px rgba(0, 0, 0, 0.18), inset 0 1px 0 rgba(251, 241, 199, 0.05)"
                    .to_owned(),
            canvas_hidden_item_bg: "rgba(40, 40, 40, 0.94)".to_owned(),
            canvas_hidden_item_hover: "rgba(60, 56, 54, 0.98)".to_owned(),
            canvas_hidden_item_selected_bg:
                "linear-gradient(135deg, rgba(215, 153, 33, 0.2), rgba(50, 48, 47, 0.98))"
                    .to_owned(),
            canvas_hidden_config_error_bg:
                "linear-gradient(180deg, rgba(204, 36, 29, 0.14), rgba(40, 40, 40, 0.98))"
                    .to_owned(),
        }
    }

    fn with_overrides(mut self, overrides: &UiColorOverridesConfig) -> Self {
        macro_rules! apply {
            ($field:ident) => {
                if let Some(value) = &overrides.$field {
                    self.$field = value.clone();
                }
            };
        }

        let accent_override = overrides.accent.clone();

        apply!(accent);
        apply!(panel);
        apply!(text);
        apply!(muted);
        apply!(canvas_bg);
        apply!(canvas_shadow);
        apply!(canvas_border);
        apply!(label_strong);
        apply!(input_bg);
        apply!(input_border);
        apply!(input_shadow);
        apply!(placeholder);
        apply!(scrollbar);
        apply!(item_bg);
        apply!(item_hover);
        apply!(item_selected_bg);
        apply!(item_selected_shadow);
        apply!(badge_bg);
        apply!(badge_border);
        apply!(badge_text);
        apply!(badge_icon_bg);
        apply!(chip_text);
        apply!(chip_bg);
        apply!(chip_border);
        apply!(config_error_bg);
        apply!(config_error_border);
        apply!(config_error_shadow);
        apply!(config_error_title);
        apply!(config_error_copy);
        apply!(canvas_hidden_input_bg);
        apply!(canvas_hidden_input_border);
        apply!(canvas_hidden_input_shadow);
        apply!(canvas_hidden_item_bg);
        apply!(canvas_hidden_item_hover);
        apply!(canvas_hidden_item_selected_bg);
        apply!(canvas_hidden_config_error_bg);

        if let Some(accent) = accent_override {
            self.apply_accent_derivatives(&accent, overrides);
        }

        self
    }

    fn apply_accent_derivatives(&mut self, accent: &str, overrides: &UiColorOverridesConfig) {
        if overrides.item_hover.is_none() {
            self.item_hover = accent_mix(accent, 14);
        }
        if overrides.item_selected_bg.is_none() {
            self.item_selected_bg = accent_gradient(accent, 28, 12);
        }
        if overrides.item_selected_shadow.is_none() {
            self.item_selected_shadow = format!(
                "0 18px 44px {}, inset 0 0 0 1px {}",
                accent_mix(accent, 18),
                accent_mix(accent, 36)
            );
        }
        if overrides.badge_bg.is_none() {
            self.badge_bg = accent_gradient(accent, 22, 8);
        }
        if overrides.badge_border.is_none() {
            self.badge_border = accent_mix(accent, 35);
        }
        if overrides.chip_border.is_none() {
            self.chip_border = accent_mix(accent, 35);
        }
        if overrides.canvas_hidden_item_hover.is_none() {
            self.canvas_hidden_item_hover = accent_mix(accent, 16);
        }
        if overrides.canvas_hidden_item_selected_bg.is_none() {
            self.canvas_hidden_item_selected_bg = accent_gradient(accent, 24, 10);
        }
    }
}

fn accent_gradient(accent: &str, start_percent: u8, end_percent: u8) -> String {
    format!(
        "linear-gradient(135deg, {}, {})",
        accent_mix(accent, start_percent),
        accent_mix(accent, end_percent)
    )
}

fn accent_mix(accent: &str, percent: u8) -> String {
    format!("color-mix(in srgb, {accent} {percent}%, transparent)")
}

/// Returns the built-in color token values used by the UI renderer.
pub fn builtin_colorscheme_token_values(name: &str) -> Option<Vec<(&'static str, String)>> {
    let colors = match name {
        "builtin_light" => ResolvedUiColors::builtin_light(),
        "builtin_dark" => ResolvedUiColors::builtin_dark(),
        _ => return None,
    };
    let serialized = serde_json::to_value(colors).ok()?;
    let values = serialized.as_object()?;
    UI_COLOR_TOKEN_NAMES
        .iter()
        .map(|token| {
            values
                .get(*token)
                .and_then(|value| value.as_str())
                .map(|value| (*token, value.to_owned()))
        })
        .collect()
}

fn resolve_theme_colors(theme: &UiConfig) -> (ResolvedUiColors, ResolvedUiColors, &'static str) {
    let builtin_light = ResolvedUiColors::builtin_light();
    let builtin_dark = ResolvedUiColors::builtin_dark();

    match theme.colorscheme() {
        "system" => (builtin_light, builtin_dark, "light dark"),
        "builtin_light" => (builtin_light.clone(), builtin_light, "light"),
        "builtin_dark" => (builtin_dark.clone(), builtin_dark, "dark"),
        other => {
            let scheme = theme.colorscheme_config(other);
            let use_light_base = matches!(scheme.base.as_deref(), Some("builtin_light"));
            let resolved = if use_light_base {
                builtin_light.with_overrides(&scheme.overrides)
            } else {
                builtin_dark.with_overrides(&scheme.overrides)
            };
            let document_color_scheme = if use_light_base { "light" } else { "dark" };
            (resolved.clone(), resolved, document_color_scheme)
        }
    }
}

/// Returns the full HTML document served into the embedded webview.
pub fn html(theme: &UiConfig, visible_rows: usize, layout_version: u64) -> String {
    let cycle_selection_value = if theme.cycle_selection {
        "true"
    } else {
        "false"
    };

    HTML_TEMPLATE
        .replace("__SHELL_CLASS__", shell_class(theme))
        .replace("__RUNX_CYCLE_SELECTION_VALUE__", cycle_selection_value)
        .replace(
            "__RUNX_FOCUS_WINDOW_SHORTCUT_VALUE__",
            &shortcut_json(&theme.shortcuts.focus_window),
        )
        .replace(
            "__RUNX_ACTIVATE_ALL_WINDOWS_SHORTCUT_VALUE__",
            &shortcut_json(&theme.shortcuts.activate_all_windows),
        )
        .replace("__RUNX_VISIBLE_ROWS_VALUE__", &visible_rows.to_string())
        .replace("__RUNX_LAYOUT_VERSION_VALUE__", &layout_version.to_string())
        .replace("__RUNX_STYLE__", &theme_css(theme))
        .replace("__RUNX_SCRIPT__", SCRIPT_SOURCE)
}

/// Returns the root shell classes implied by the current canvas mode.
pub fn shell_class(theme: &UiConfig) -> &'static str {
    if theme.canvas.show {
        "shell"
    } else {
        "shell canvas-hidden"
    }
}

/// Returns the JSON value consumed by the frontend shortcut matcher.
pub fn shortcut_json(shortcut: &Option<crate::config::UiShortcutConfig>) -> String {
    match serde_json::to_string(shortcut) {
        Ok(value) => value,
        Err(_) => "null".to_owned(),
    }
}

/// Returns the theme-expanded CSS used by the embedded webview.
pub fn theme_css(theme: &UiConfig) -> String {
    let (light, dark, document_color_scheme) = resolve_theme_colors(theme);
    let scale = theme.scale;
    let header_display = if theme.show_header { "flex" } else { "none" };
    let canvas_display = if theme.canvas.show { "block" } else { "none" };
    let ui_scale = format_float(scale);
    let canvas_radius = scaled_px(theme.canvas.radius, scale);
    let canvas_background_opacity = format!("{}", theme.canvas.background_opacity);
    let canvas_chrome_opacity = format!("{}", theme.canvas.chrome_opacity);
    let entry_opacity = format!("{}", theme.entries.opacity);
    let label_font_size = scaled_px(theme.font_sizes.label, scale);
    let input_font_size = scaled_px(theme.font_sizes.input, scale);
    let title_font_size = scaled_px(theme.font_sizes.title, scale);
    let subtitle_font_size = scaled_px(theme.font_sizes.subtitle, scale);
    let badge_font_size = scaled_px(theme.font_sizes.badge, scale);
    let accelerator_font_size = scaled_px(theme.font_sizes.accelerator, scale);
    let config_error_title_font_size = scaled_px(theme.font_sizes.config_error_title, scale);
    let config_error_body_font_size = scaled_px(theme.font_sizes.config_error_body, scale);
    let section_gap = scaled_px(theme.layout.section_gap, scale);
    let input_padding_y = scaled_px(theme.layout.input_padding_y, scale);
    let input_padding_x = scaled_px(theme.layout.input_padding_x, scale);
    let input_radius = scaled_px(theme.layout.input_radius, scale);
    let list_gap = scaled_px(theme.layout.list_gap, scale);
    let entry_padding_y = scaled_px(theme.layout.entry_padding_y, scale);
    let entry_padding_x = scaled_px(theme.layout.entry_padding_x, scale);
    let entry_gap = scaled_px(theme.layout.entry_gap, scale);
    let row_radius = scaled_px(theme.layout.row_radius, scale);
    let badge_size = scaled_px(theme.layout.badge_size, scale);
    let badge_radius = scaled_px(theme.layout.badge_radius, scale);
    let icon_size = scaled_px(theme.layout.icon_size, scale);

    let replacements = [
        ("__LIGHT_ACCENT__", light.accent.as_str()),
        ("__LIGHT_PANEL__", light.panel.as_str()),
        ("__LIGHT_TEXT__", light.text.as_str()),
        ("__LIGHT_MUTED__", light.muted.as_str()),
        ("__LIGHT_CANVAS_BG__", light.canvas_bg.as_str()),
        ("__LIGHT_CANVAS_SHADOW__", light.canvas_shadow.as_str()),
        ("__LIGHT_CANVAS_BORDER__", light.canvas_border.as_str()),
        ("__LIGHT_LABEL_STRONG__", light.label_strong.as_str()),
        ("__LIGHT_INPUT_BG__", light.input_bg.as_str()),
        ("__LIGHT_INPUT_BORDER__", light.input_border.as_str()),
        ("__LIGHT_INPUT_SHADOW__", light.input_shadow.as_str()),
        ("__LIGHT_PLACEHOLDER__", light.placeholder.as_str()),
        ("__LIGHT_SCROLLBAR__", light.scrollbar.as_str()),
        ("__LIGHT_ITEM_BG__", light.item_bg.as_str()),
        ("__LIGHT_ITEM_HOVER__", light.item_hover.as_str()),
        (
            "__LIGHT_ITEM_SELECTED_BG__",
            light.item_selected_bg.as_str(),
        ),
        (
            "__LIGHT_ITEM_SELECTED_SHADOW__",
            light.item_selected_shadow.as_str(),
        ),
        ("__LIGHT_BADGE_BG__", light.badge_bg.as_str()),
        ("__LIGHT_BADGE_BORDER__", light.badge_border.as_str()),
        ("__LIGHT_BADGE_TEXT__", light.badge_text.as_str()),
        ("__LIGHT_BADGE_ICON_BG__", light.badge_icon_bg.as_str()),
        ("__LIGHT_CHIP_TEXT__", light.chip_text.as_str()),
        ("__LIGHT_CHIP_BG__", light.chip_bg.as_str()),
        ("__LIGHT_CHIP_BORDER__", light.chip_border.as_str()),
        ("__LIGHT_CONFIG_ERROR_BG__", light.config_error_bg.as_str()),
        (
            "__LIGHT_CONFIG_ERROR_BORDER__",
            light.config_error_border.as_str(),
        ),
        (
            "__LIGHT_CONFIG_ERROR_SHADOW__",
            light.config_error_shadow.as_str(),
        ),
        (
            "__LIGHT_CONFIG_ERROR_TITLE__",
            light.config_error_title.as_str(),
        ),
        (
            "__LIGHT_CONFIG_ERROR_COPY__",
            light.config_error_copy.as_str(),
        ),
        (
            "__LIGHT_CANVAS_HIDDEN_INPUT_BG__",
            light.canvas_hidden_input_bg.as_str(),
        ),
        (
            "__LIGHT_CANVAS_HIDDEN_INPUT_BORDER__",
            light.canvas_hidden_input_border.as_str(),
        ),
        (
            "__LIGHT_CANVAS_HIDDEN_INPUT_SHADOW__",
            light.canvas_hidden_input_shadow.as_str(),
        ),
        (
            "__LIGHT_CANVAS_HIDDEN_ITEM_BG__",
            light.canvas_hidden_item_bg.as_str(),
        ),
        (
            "__LIGHT_CANVAS_HIDDEN_ITEM_HOVER__",
            light.canvas_hidden_item_hover.as_str(),
        ),
        (
            "__LIGHT_CANVAS_HIDDEN_ITEM_SELECTED_BG__",
            light.canvas_hidden_item_selected_bg.as_str(),
        ),
        (
            "__LIGHT_CANVAS_HIDDEN_CONFIG_ERROR_BG__",
            light.canvas_hidden_config_error_bg.as_str(),
        ),
        ("__DARK_ACCENT__", dark.accent.as_str()),
        ("__DARK_PANEL__", dark.panel.as_str()),
        ("__DARK_TEXT__", dark.text.as_str()),
        ("__DARK_MUTED__", dark.muted.as_str()),
        ("__DARK_CANVAS_BG__", dark.canvas_bg.as_str()),
        ("__DARK_CANVAS_SHADOW__", dark.canvas_shadow.as_str()),
        ("__DARK_CANVAS_BORDER__", dark.canvas_border.as_str()),
        ("__DARK_LABEL_STRONG__", dark.label_strong.as_str()),
        ("__DARK_INPUT_BG__", dark.input_bg.as_str()),
        ("__DARK_INPUT_BORDER__", dark.input_border.as_str()),
        ("__DARK_INPUT_SHADOW__", dark.input_shadow.as_str()),
        ("__DARK_PLACEHOLDER__", dark.placeholder.as_str()),
        ("__DARK_SCROLLBAR__", dark.scrollbar.as_str()),
        ("__DARK_ITEM_BG__", dark.item_bg.as_str()),
        ("__DARK_ITEM_HOVER__", dark.item_hover.as_str()),
        ("__DARK_ITEM_SELECTED_BG__", dark.item_selected_bg.as_str()),
        (
            "__DARK_ITEM_SELECTED_SHADOW__",
            dark.item_selected_shadow.as_str(),
        ),
        ("__DARK_BADGE_BG__", dark.badge_bg.as_str()),
        ("__DARK_BADGE_BORDER__", dark.badge_border.as_str()),
        ("__DARK_BADGE_TEXT__", dark.badge_text.as_str()),
        ("__DARK_BADGE_ICON_BG__", dark.badge_icon_bg.as_str()),
        ("__DARK_CHIP_TEXT__", dark.chip_text.as_str()),
        ("__DARK_CHIP_BG__", dark.chip_bg.as_str()),
        ("__DARK_CHIP_BORDER__", dark.chip_border.as_str()),
        ("__DARK_CONFIG_ERROR_BG__", dark.config_error_bg.as_str()),
        (
            "__DARK_CONFIG_ERROR_BORDER__",
            dark.config_error_border.as_str(),
        ),
        (
            "__DARK_CONFIG_ERROR_SHADOW__",
            dark.config_error_shadow.as_str(),
        ),
        (
            "__DARK_CONFIG_ERROR_TITLE__",
            dark.config_error_title.as_str(),
        ),
        (
            "__DARK_CONFIG_ERROR_COPY__",
            dark.config_error_copy.as_str(),
        ),
        (
            "__DARK_CANVAS_HIDDEN_INPUT_BG__",
            dark.canvas_hidden_input_bg.as_str(),
        ),
        (
            "__DARK_CANVAS_HIDDEN_INPUT_BORDER__",
            dark.canvas_hidden_input_border.as_str(),
        ),
        (
            "__DARK_CANVAS_HIDDEN_INPUT_SHADOW__",
            dark.canvas_hidden_input_shadow.as_str(),
        ),
        (
            "__DARK_CANVAS_HIDDEN_ITEM_BG__",
            dark.canvas_hidden_item_bg.as_str(),
        ),
        (
            "__DARK_CANVAS_HIDDEN_ITEM_HOVER__",
            dark.canvas_hidden_item_hover.as_str(),
        ),
        (
            "__DARK_CANVAS_HIDDEN_ITEM_SELECTED_BG__",
            dark.canvas_hidden_item_selected_bg.as_str(),
        ),
        (
            "__DARK_CANVAS_HIDDEN_CONFIG_ERROR_BG__",
            dark.canvas_hidden_config_error_bg.as_str(),
        ),
        ("__FONT_FAMILY__", theme.font_family.as_str()),
        ("__UI_SCALE__", ui_scale.as_str()),
        ("__DOCUMENT_COLOR_SCHEME__", document_color_scheme),
        ("__HEADER_DISPLAY__", header_display),
        ("__CANVAS_DISPLAY__", canvas_display),
        ("__CANVAS_RADIUS__", canvas_radius.as_str()),
        (
            "__CANVAS_BACKGROUND_OPACITY__",
            canvas_background_opacity.as_str(),
        ),
        ("__CANVAS_CHROME_OPACITY__", canvas_chrome_opacity.as_str()),
        ("__ENTRY_OPACITY__", entry_opacity.as_str()),
        ("__LABEL_FONT_SIZE__", label_font_size.as_str()),
        ("__INPUT_FONT_SIZE__", input_font_size.as_str()),
        ("__TITLE_FONT_SIZE__", title_font_size.as_str()),
        ("__SUBTITLE_FONT_SIZE__", subtitle_font_size.as_str()),
        ("__BADGE_FONT_SIZE__", badge_font_size.as_str()),
        ("__ACCELERATOR_FONT_SIZE__", accelerator_font_size.as_str()),
        (
            "__CONFIG_ERROR_TITLE_FONT_SIZE__",
            config_error_title_font_size.as_str(),
        ),
        (
            "__CONFIG_ERROR_BODY_FONT_SIZE__",
            config_error_body_font_size.as_str(),
        ),
        ("__SECTION_GAP__", section_gap.as_str()),
        ("__INPUT_PADDING_Y__", input_padding_y.as_str()),
        ("__INPUT_PADDING_X__", input_padding_x.as_str()),
        ("__INPUT_RADIUS__", input_radius.as_str()),
        ("__LIST_GAP__", list_gap.as_str()),
        ("__ENTRY_PADDING_Y__", entry_padding_y.as_str()),
        ("__ENTRY_PADDING_X__", entry_padding_x.as_str()),
        ("__ENTRY_GAP__", entry_gap.as_str()),
        ("__ROW_RADIUS__", row_radius.as_str()),
        ("__BADGE_SIZE__", badge_size.as_str()),
        ("__BADGE_RADIUS__", badge_radius.as_str()),
        ("__ICON_SIZE__", icon_size.as_str()),
    ];

    let mut css = STYLE_TEMPLATE.to_owned();
    for (token, value) in replacements {
        css = css.replace(token, value);
    }
    css
}

fn scaled_px(value: u16, scale: f64) -> String {
    format!("{}px", format_float(f64::from(value) * scale))
}

fn format_float(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < f64::EPSILON {
        format!("{rounded:.0}")
    } else {
        let mut rendered = format!("{rounded:.2}");
        while rendered.contains('.') && rendered.ends_with('0') {
            rendered.pop();
        }
        if rendered.ends_with('.') {
            rendered.pop();
        }
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::{html, theme_css};
    use crate::config::{UiColorOverridesConfig, UiColorschemeConfig, UiConfig};

    #[test]
    fn theme_css_includes_configured_font_sizes() {
        let mut theme = UiConfig {
            show_header: false,
            ..UiConfig::default()
        };
        theme.scale = 1.25;
        theme.canvas.show = false;
        theme.canvas.radius = 20;
        theme.canvas.background_opacity = 0.91;
        theme.canvas.chrome_opacity = 0.72;
        theme.entries.opacity = 0.64;
        theme.font_sizes.input = 34;
        theme.font_sizes.title = 18;
        theme.font_sizes.config_error_body = 17;
        theme.layout.section_gap = 16;
        theme.layout.input_radius = 22;
        theme.layout.badge_size = 52;

        let css = theme_css(&theme);

        assert!(css.contains("--header-display: none;"));
        assert!(css.contains("--ui-scale: 1.25;"));
        assert!(css.contains("--canvas-display: none;"));
        assert!(css.contains("--canvas-radius: 25px;"));
        assert!(css.contains("--canvas-background-opacity: 0.91;"));
        assert!(css.contains("--canvas-chrome-opacity: 0.72;"));
        assert!(css.contains("opacity: var(--canvas-background-opacity);"));
        assert!(css.contains("opacity: var(--canvas-chrome-opacity);"));
        assert!(css.contains("--entry-opacity: 0.64;"));
        assert!(css.contains("--input-font-size: 42.5px;"));
        assert!(css.contains("--title-font-size: 22.5px;"));
        assert!(css.contains("--config-error-body-font-size: 21.25px;"));
        assert!(css.contains("--section-gap: 20px;"));
        assert!(css.contains("--input-radius: 27.5px;"));
        assert!(css.contains("--badge-size: 65px;"));
    }

    #[test]
    fn builtin_colorscheme_tables_do_not_override_builtin_palettes() {
        let mut theme = UiConfig::default();
        theme.colorschemes.insert(
            "builtin_light".to_owned(),
            UiColorschemeConfig {
                base: None,
                overrides: UiColorOverridesConfig {
                    canvas_bg: Some("linear-gradient(180deg, #111111, #222222)".to_owned()),
                    ..UiColorOverridesConfig::default()
                },
            },
        );

        let css = theme_css(&theme);

        assert!(!css.contains("linear-gradient(180deg, #111111, #222222)"));
        assert!(css.contains("--canvas-bg: linear-gradient(180deg, #fbf1c7, #f2e5bc);"));
    }

    #[test]
    fn explicit_custom_colorscheme_is_pinned_in_both_light_and_dark_css() {
        let mut theme = UiConfig {
            colorscheme: "gruvbox".to_owned(),
            ..UiConfig::default()
        };
        theme.colorschemes.insert(
            "gruvbox".to_owned(),
            UiColorschemeConfig {
                base: Some("builtin_dark".to_owned()),
                overrides: UiColorOverridesConfig {
                    accent: Some("#fabd2f".to_owned()),
                    panel: Some("#282828".to_owned()),
                    ..UiColorOverridesConfig::default()
                },
            },
        );

        let css = theme_css(&theme);

        assert!(css.contains("--document-color-scheme: dark;"));
        assert!(css.contains("--accent: #fabd2f;"));
        assert!(css.contains("--panel: #282828;"));
        assert!(css.matches("--accent: #fabd2f;").count() >= 2);
    }

    #[test]
    fn custom_accent_derives_highlight_surfaces() {
        let mut theme = UiConfig {
            colorscheme: "gruvbox".to_owned(),
            ..UiConfig::default()
        };
        theme.colorschemes.insert(
            "gruvbox".to_owned(),
            UiColorschemeConfig {
                base: Some("builtin_dark".to_owned()),
                overrides: UiColorOverridesConfig {
                    accent: Some("#ffcc00".to_owned()),
                    ..UiColorOverridesConfig::default()
                },
            },
        );

        let css = theme_css(&theme);

        assert!(css.contains("--accent: #ffcc00;"));
        assert!(css.contains("--item-hover: color-mix(in srgb, #ffcc00 14%, transparent);"));
        assert!(css.contains("--item-selected-bg: linear-gradient(135deg, color-mix(in srgb, #ffcc00 28%, transparent), color-mix(in srgb, #ffcc00 12%, transparent));"));
        assert!(css.contains("--item-selected-shadow: 0 18px 44px color-mix(in srgb, #ffcc00 18%, transparent), inset 0 0 0 1px color-mix(in srgb, #ffcc00 36%, transparent);"));
        assert!(css.contains("--badge-bg: linear-gradient(135deg, color-mix(in srgb, #ffcc00 22%, transparent), color-mix(in srgb, #ffcc00 8%, transparent));"));
        assert!(css.contains("--badge-border: color-mix(in srgb, #ffcc00 35%, transparent);"));
        assert!(css.contains("--chip-border: color-mix(in srgb, #ffcc00 35%, transparent);"));
        assert!(
            css.contains(
                "--canvas-hidden-item-hover: color-mix(in srgb, #ffcc00 16%, transparent);"
            )
        );
        assert!(css.contains("--canvas-hidden-item-selected-bg: linear-gradient(135deg, color-mix(in srgb, #ffcc00 24%, transparent), color-mix(in srgb, #ffcc00 10%, transparent));"));
    }

    #[test]
    fn explicit_highlight_tokens_override_accent_derivatives() {
        let mut theme = UiConfig {
            colorscheme: "gruvbox".to_owned(),
            ..UiConfig::default()
        };
        theme.colorschemes.insert(
            "gruvbox".to_owned(),
            UiColorschemeConfig {
                base: Some("builtin_dark".to_owned()),
                overrides: UiColorOverridesConfig {
                    accent: Some("#ffcc00".to_owned()),
                    item_hover: Some("#101010".to_owned()),
                    item_selected_bg: Some("#111111".to_owned()),
                    item_selected_shadow: Some("0 0 0 1px #222222".to_owned()),
                    badge_bg: Some("#333333".to_owned()),
                    badge_border: Some("#444444".to_owned()),
                    chip_border: Some("#555555".to_owned()),
                    canvas_hidden_item_hover: Some("#606060".to_owned()),
                    canvas_hidden_item_selected_bg: Some("#666666".to_owned()),
                    ..UiColorOverridesConfig::default()
                },
            },
        );

        let css = theme_css(&theme);

        assert!(css.contains("--accent: #ffcc00;"));
        assert!(css.contains("--item-hover: #101010;"));
        assert!(css.contains("--item-selected-bg: #111111;"));
        assert!(css.contains("--item-selected-shadow: 0 0 0 1px #222222;"));
        assert!(css.contains("--badge-bg: #333333;"));
        assert!(css.contains("--badge-border: #444444;"));
        assert!(css.contains("--chip-border: #555555;"));
        assert!(css.contains("--canvas-hidden-item-hover: #606060;"));
        assert!(css.contains("--canvas-hidden-item-selected-bg: #666666;"));
        assert!(!css.contains("--item-hover: color-mix(in srgb, #ffcc00 14%, transparent);"));
        assert!(!css.contains(
            "--item-selected-bg: linear-gradient(135deg, color-mix(in srgb, #ffcc00 28%"
        ));
    }

    #[test]
    fn custom_canvas_bg_override_updates_visible_canvas_background() {
        let mut theme = UiConfig {
            colorscheme: "gruvbox".to_owned(),
            ..UiConfig::default()
        };
        theme.colorschemes.insert(
            "gruvbox".to_owned(),
            UiColorschemeConfig {
                base: Some("builtin_dark".to_owned()),
                overrides: UiColorOverridesConfig {
                    canvas_bg: Some("#0000ff".to_owned()),
                    ..UiColorOverridesConfig::default()
                },
            },
        );

        let css = theme_css(&theme);

        assert!(css.contains("--canvas-bg: #0000ff;"));
    }

    #[test]
    fn system_colorscheme_keeps_light_dark_pairing() {
        let theme = UiConfig::default();

        let css = theme_css(&theme);

        assert!(css.contains("--document-color-scheme: light dark;"));
        assert!(!css.contains("--accent: __LIGHT_ACCENT__"));
        assert!(css.contains("--accent: #b57614;"));
        assert!(css.contains("@media (prefers-color-scheme: dark)"));
        assert!(css.contains("--accent: #d79921;"));
    }

    #[test]
    fn html_marks_shell_as_canvas_hidden_when_canvas_is_disabled() {
        let mut theme = UiConfig::default();
        theme.canvas.show = false;

        let document = html(&theme, 5, 0);

        assert!(document.contains(r#"<main class="shell canvas-hidden">"#));
    }

    #[test]
    fn html_exposes_cycle_selection_flag() {
        let theme = UiConfig {
            cycle_selection: true,
            ..UiConfig::default()
        };

        let document = html(&theme, 5, 0);

        assert!(document.contains("window.__RUNX_CYCLE_SELECTION__ = true;"));
    }

    #[test]
    fn html_exposes_window_action_shortcuts() {
        let document = html(&UiConfig::default(), 5, 7);

        assert!(document.contains(
            r#"window.__RUNX_FOCUS_WINDOW_SHORTCUT__ = {"key":"Enter","code":null,"alt":false,"ctrl":false,"meta":false,"shift":false};"#
        ));
        assert!(document.contains(
            r#"window.__RUNX_ACTIVATE_ALL_WINDOWS_SHORTCUT__ = {"key":"Enter","code":null,"alt":true,"ctrl":false,"meta":false,"shift":false};"#
        ));
        assert!(document.contains("window.__RUNX_VISIBLE_ROWS__ = 5;"));
        assert!(document.contains("window.__RUNX_LAYOUT_VERSION__ = 7;"));
    }
}
