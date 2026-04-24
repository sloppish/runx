//! Embeds the static frontend templates into a themed HTML document.

use crate::config::{UiColorOverridesConfig, UiConfig};

const HTML_TEMPLATE: &str = include_str!("../ui/index.html");
const STYLE_TEMPLATE: &str = include_str!("../ui/styles.css");
const SCRIPT_SOURCE: &str = include_str!("../ui/app.js");

#[derive(Clone)]
struct ResolvedUiColors {
    accent: String,
    background: String,
    panel: String,
    text: String,
    muted: String,
    shell_bg: String,
    shell_shadow: String,
    shell_border: String,
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
            accent: "#c77b49".to_owned(),
            background: "#f3ede5".to_owned(),
            panel: "#fffaf3".to_owned(),
            text: "#1f1a16".to_owned(),
            muted: "#756759".to_owned(),
            shell_bg: "linear-gradient(180deg, rgb(255 255 255), rgb(252 246 238))".to_owned(),
            shell_shadow: "inset 0 1px 0 rgba(255, 255, 255, 0.72)".to_owned(),
            shell_border: "rgba(140, 102, 67, 0.16)".to_owned(),
            label_strong: "color-mix(in srgb, var(--text) 88%, white)".to_owned(),
            input_bg: "linear-gradient(180deg, rgba(255,255,255,0.92), rgba(248,240,230,0.9))"
                .to_owned(),
            input_border: "rgba(140, 102, 67, 0.14)".to_owned(),
            input_shadow: "inset 0 1px 0 rgba(255,255,255,0.72)".to_owned(),
            placeholder: "color-mix(in srgb, var(--muted) 78%, white)".to_owned(),
            scrollbar: "rgba(134, 98, 66, 0.18)".to_owned(),
            item_bg: "rgba(255,255,255,0.45)".to_owned(),
            item_hover: "rgba(255,255,255,0.86)".to_owned(),
            item_selected_bg:
                "linear-gradient(135deg, rgba(199, 123, 73, 0.15), rgba(255,255,255,0.94))"
                    .to_owned(),
            item_selected_shadow:
                "0 18px 44px rgba(139, 87, 46, 0.14), inset 0 0 0 1px rgba(199, 123, 73, 0.22)"
                    .to_owned(),
            badge_bg:
                "linear-gradient(180deg, color-mix(in srgb, var(--accent) 18%, white), rgba(255,255,255,0.95))"
                    .to_owned(),
            badge_border: "rgba(199, 123, 73, 0.18)".to_owned(),
            badge_text: "color-mix(in srgb, var(--accent) 72%, black)".to_owned(),
            badge_icon_bg: "rgba(15, 22, 31, 0.78)".to_owned(),
            chip_text: "color-mix(in srgb, var(--muted) 80%, white)".to_owned(),
            chip_bg: "rgba(255,255,255,0.68)".to_owned(),
            chip_border: "rgba(140, 102, 67, 0.12)".to_owned(),
            config_error_bg:
                "linear-gradient(180deg, rgba(199, 123, 73, 0.12), rgba(255,255,255,0.04))"
                    .to_owned(),
            config_error_border: "color-mix(in srgb, var(--accent) 30%, transparent)".to_owned(),
            config_error_shadow:
                "0 18px 44px rgba(139, 87, 46, 0.12), inset 0 1px 0 rgba(255,255,255,0.06)"
                    .to_owned(),
            config_error_title: "color-mix(in srgb, var(--text) 92%, white)".to_owned(),
            config_error_copy: "color-mix(in srgb, var(--text) 84%, white)".to_owned(),
            canvas_hidden_input_bg:
                "linear-gradient(180deg, color-mix(in srgb, var(--panel) 98%, white), color-mix(in srgb, var(--panel) 96%, black))"
                    .to_owned(),
            canvas_hidden_input_border:
                "color-mix(in srgb, var(--text) 14%, transparent)".to_owned(),
            canvas_hidden_input_shadow:
                "0 10px 28px rgba(0, 0, 0, 0.12), inset 0 1px 0 rgba(255,255,255,0.08)"
                    .to_owned(),
            canvas_hidden_item_bg:
                "color-mix(in srgb, var(--panel) 94%, transparent)".to_owned(),
            canvas_hidden_item_hover:
                "color-mix(in srgb, var(--panel) 98%, var(--accent) 2%)".to_owned(),
            canvas_hidden_item_selected_bg:
                "linear-gradient(135deg, color-mix(in srgb, var(--accent) 22%, var(--panel)), color-mix(in srgb, var(--panel) 98%, white))"
                    .to_owned(),
            canvas_hidden_config_error_bg:
                "linear-gradient(180deg, color-mix(in srgb, var(--accent) 14%, var(--panel)), color-mix(in srgb, var(--panel) 96%, black))"
                    .to_owned(),
        }
    }

    fn builtin_dark() -> Self {
        Self {
            accent: "#d7a17b".to_owned(),
            background: "#10151d".to_owned(),
            panel: "#18202b".to_owned(),
            text: "#eef2fb".to_owned(),
            muted: "#99a6bc".to_owned(),
            shell_bg: "linear-gradient(180deg, rgb(28 37 50), rgb(16 22 31))".to_owned(),
            shell_shadow: "inset 0 1px 0 rgba(255, 255, 255, 0.05)".to_owned(),
            shell_border: "rgba(128, 164, 214, 0.18)".to_owned(),
            label_strong: "color-mix(in srgb, var(--text) 94%, white)".to_owned(),
            input_bg: "linear-gradient(180deg, rgba(24, 31, 43, 0.98), rgba(18, 25, 35, 0.98))"
                .to_owned(),
            input_border: "rgba(128, 164, 214, 0.14)".to_owned(),
            input_shadow: "inset 0 1px 0 rgba(255,255,255,0.03)".to_owned(),
            placeholder: "color-mix(in srgb, var(--muted) 88%, black)".to_owned(),
            scrollbar: "rgba(128, 164, 214, 0.26)".to_owned(),
            item_bg: "rgba(180, 206, 244, 0.035)".to_owned(),
            item_hover: "rgba(162, 195, 242, 0.08)".to_owned(),
            item_selected_bg:
                "linear-gradient(135deg, rgba(103, 148, 210, 0.24), rgba(31, 41, 56, 0.98))"
                    .to_owned(),
            item_selected_shadow:
                "0 18px 44px rgba(0, 0, 0, 0.22), inset 0 0 0 1px rgba(117, 160, 222, 0.24)"
                    .to_owned(),
            badge_bg:
                "linear-gradient(180deg, rgba(118, 154, 206, 0.18), rgba(255,255,255,0.02))"
                    .to_owned(),
            badge_border: "rgba(128, 164, 214, 0.18)".to_owned(),
            badge_text: "color-mix(in srgb, #b9d4f4 86%, white)".to_owned(),
            badge_icon_bg: "rgba(255,255,255,0.66)".to_owned(),
            chip_text: "color-mix(in srgb, var(--muted) 86%, white)".to_owned(),
            chip_bg: "rgba(150, 182, 230, 0.05)".to_owned(),
            chip_border: "rgba(128, 164, 214, 0.14)".to_owned(),
            config_error_bg:
                "linear-gradient(180deg, rgba(103, 148, 210, 0.18), rgba(16,22,31,0.22))"
                    .to_owned(),
            config_error_border: "color-mix(in srgb, var(--accent) 24%, transparent)".to_owned(),
            config_error_shadow:
                "0 18px 44px rgba(0, 0, 0, 0.22), inset 0 1px 0 rgba(255,255,255,0.03)"
                    .to_owned(),
            config_error_title: "color-mix(in srgb, var(--text) 94%, white)".to_owned(),
            config_error_copy: "color-mix(in srgb, var(--text) 86%, white)".to_owned(),
            canvas_hidden_input_bg:
                "linear-gradient(180deg, color-mix(in srgb, var(--panel) 98%, white), color-mix(in srgb, var(--panel) 96%, black))"
                    .to_owned(),
            canvas_hidden_input_border:
                "color-mix(in srgb, var(--text) 14%, transparent)".to_owned(),
            canvas_hidden_input_shadow:
                "0 10px 28px rgba(0, 0, 0, 0.12), inset 0 1px 0 rgba(255,255,255,0.08)"
                    .to_owned(),
            canvas_hidden_item_bg:
                "color-mix(in srgb, var(--panel) 94%, transparent)".to_owned(),
            canvas_hidden_item_hover:
                "color-mix(in srgb, var(--panel) 98%, var(--accent) 2%)".to_owned(),
            canvas_hidden_item_selected_bg:
                "linear-gradient(135deg, color-mix(in srgb, var(--accent) 22%, var(--panel)), color-mix(in srgb, var(--panel) 98%, white))"
                    .to_owned(),
            canvas_hidden_config_error_bg:
                "linear-gradient(180deg, color-mix(in srgb, var(--accent) 14%, var(--panel)), color-mix(in srgb, var(--panel) 96%, black))"
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

        apply!(accent);
        apply!(background);
        apply!(panel);
        apply!(text);
        apply!(muted);
        apply!(shell_bg);
        apply!(shell_shadow);
        apply!(shell_border);
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
        self
    }
}

fn resolve_theme_colors(theme: &UiConfig) -> (ResolvedUiColors, ResolvedUiColors, &'static str) {
    let builtin_light = ResolvedUiColors::builtin_light()
        .with_overrides(&theme.colorscheme_config("builtin_light").overrides);
    let builtin_dark = ResolvedUiColors::builtin_dark()
        .with_overrides(&theme.colorscheme_config("builtin_dark").overrides);

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
pub fn html(theme: &UiConfig) -> String {
    let shell_class = if theme.canvas.show {
        "shell"
    } else {
        "shell canvas-hidden"
    };
    let cycle_selection_value = if theme.cycle_selection {
        "true"
    } else {
        "false"
    };

    HTML_TEMPLATE
        .replace("__SHELL_CLASS__", shell_class)
        .replace("__RUNX_CYCLE_SELECTION_VALUE__", cycle_selection_value)
        .replace(
            "__RUNX_FOCUS_WINDOW_SHORTCUT_VALUE__",
            &shortcut_json(&theme.shortcuts.focus_window),
        )
        .replace(
            "__RUNX_ACTIVATE_ALL_WINDOWS_SHORTCUT_VALUE__",
            &shortcut_json(&theme.shortcuts.activate_all_windows),
        )
        .replace("__RUNX_STYLE__", &theme_css(theme))
        .replace("__RUNX_SCRIPT__", SCRIPT_SOURCE)
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
    let header_display = if theme.show_header { "flex" } else { "none" };
    let canvas_display = if theme.canvas.show { "block" } else { "none" };
    let canvas_radius = format!("{}px", theme.canvas.radius);
    let canvas_opacity = format!("{}", theme.canvas.opacity);
    let canvas_background_opacity = format!("{}", theme.canvas.background_opacity);
    let entry_opacity = format!("{}", theme.entries.opacity);
    let label_font_size = format!("{}px", theme.font_sizes.label);
    let input_font_size = format!("{}px", theme.font_sizes.input);
    let title_font_size = format!("{}px", theme.font_sizes.title);
    let subtitle_font_size = format!("{}px", theme.font_sizes.subtitle);
    let badge_font_size = format!("{}px", theme.font_sizes.badge);
    let accelerator_font_size = format!("{}px", theme.font_sizes.accelerator);
    let config_error_title_font_size = format!("{}px", theme.font_sizes.config_error_title);
    let config_error_body_font_size = format!("{}px", theme.font_sizes.config_error_body);
    let section_gap = format!("{}px", theme.layout.section_gap);
    let input_padding_y = format!("{}px", theme.layout.input_padding_y);
    let input_padding_x = format!("{}px", theme.layout.input_padding_x);
    let input_radius = format!("{}px", theme.layout.input_radius);
    let list_gap = format!("{}px", theme.layout.list_gap);
    let entry_padding_y = format!("{}px", theme.layout.entry_padding_y);
    let entry_padding_x = format!("{}px", theme.layout.entry_padding_x);
    let entry_gap = format!("{}px", theme.layout.entry_gap);
    let row_radius = format!("{}px", theme.layout.row_radius);
    let badge_size = format!("{}px", theme.layout.badge_size);
    let badge_radius = format!("{}px", theme.layout.badge_radius);
    let icon_size = format!("{}px", theme.layout.icon_size);

    let replacements = [
        ("__LIGHT_ACCENT__", light.accent.as_str()),
        ("__LIGHT_BACKGROUND__", light.background.as_str()),
        ("__LIGHT_PANEL__", light.panel.as_str()),
        ("__LIGHT_TEXT__", light.text.as_str()),
        ("__LIGHT_MUTED__", light.muted.as_str()),
        ("__LIGHT_SHELL_BG__", light.shell_bg.as_str()),
        ("__LIGHT_SHELL_SHADOW__", light.shell_shadow.as_str()),
        ("__LIGHT_SHELL_BORDER__", light.shell_border.as_str()),
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
        ("__DARK_BACKGROUND__", dark.background.as_str()),
        ("__DARK_PANEL__", dark.panel.as_str()),
        ("__DARK_TEXT__", dark.text.as_str()),
        ("__DARK_MUTED__", dark.muted.as_str()),
        ("__DARK_SHELL_BG__", dark.shell_bg.as_str()),
        ("__DARK_SHELL_SHADOW__", dark.shell_shadow.as_str()),
        ("__DARK_SHELL_BORDER__", dark.shell_border.as_str()),
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
        ("__DOCUMENT_COLOR_SCHEME__", document_color_scheme),
        ("__HEADER_DISPLAY__", header_display),
        ("__CANVAS_DISPLAY__", canvas_display),
        ("__CANVAS_RADIUS__", canvas_radius.as_str()),
        ("__CANVAS_OPACITY__", canvas_opacity.as_str()),
        (
            "__CANVAS_BACKGROUND_OPACITY__",
            canvas_background_opacity.as_str(),
        ),
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
        theme.canvas.show = false;
        theme.canvas.radius = 20;
        theme.canvas.opacity = 0.72;
        theme.canvas.background_opacity = 0.91;
        theme.entries.opacity = 0.64;
        theme
            .colorschemes
            .get_mut("builtin_light")
            .expect("builtin light scheme should exist")
            .overrides
            .shell_bg = Some("linear-gradient(180deg, #111111, #222222)".to_owned());
        theme
            .colorschemes
            .get_mut("builtin_dark")
            .expect("builtin dark scheme should exist")
            .overrides
            .badge_icon_bg = Some("rgba(4, 5, 6, 0.7)".to_owned());
        theme.font_sizes.input = 34;
        theme.font_sizes.title = 18;
        theme.font_sizes.config_error_body = 17;
        theme.layout.section_gap = 16;
        theme.layout.input_radius = 22;
        theme.layout.badge_size = 52;

        let css = theme_css(&theme);

        assert!(css.contains("--header-display: none;"));
        assert!(css.contains("--canvas-display: none;"));
        assert!(css.contains("--canvas-radius: 20px;"));
        assert!(css.contains("--canvas-opacity: 0.72;"));
        assert!(css.contains("--canvas-background-opacity: 0.91;"));
        assert!(css.contains("--entry-opacity: 0.64;"));
        assert!(css.contains("--shell-bg: linear-gradient(180deg, #111111, #222222);"));
        assert!(css.contains("--badge-icon-bg: rgba(4, 5, 6, 0.7);"));
        assert!(css.contains("--input-font-size: 34px;"));
        assert!(css.contains("--title-font-size: 18px;"));
        assert!(css.contains("--config-error-body-font-size: 17px;"));
        assert!(css.contains("--section-gap: 16px;"));
        assert!(css.contains("--input-radius: 22px;"));
        assert!(css.contains("--badge-size: 52px;"));
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
    fn system_colorscheme_keeps_light_dark_pairing() {
        let theme = UiConfig::default();

        let css = theme_css(&theme);

        assert!(css.contains("--document-color-scheme: light dark;"));
        assert!(!css.contains("--accent: __LIGHT_ACCENT__"));
        assert!(css.contains("--accent: #c77b49;"));
        assert!(css.contains("@media (prefers-color-scheme: dark)"));
        assert!(css.contains("--accent: #d7a17b;"));
    }

    #[test]
    fn html_marks_shell_as_canvas_hidden_when_canvas_is_disabled() {
        let mut theme = UiConfig::default();
        theme.canvas.show = false;

        let document = html(&theme);

        assert!(document.contains(r#"<main class="shell canvas-hidden">"#));
    }

    #[test]
    fn html_exposes_cycle_selection_flag() {
        let theme = UiConfig {
            cycle_selection: true,
            ..UiConfig::default()
        };

        let document = html(&theme);

        assert!(document.contains("window.__RUNX_CYCLE_SELECTION__ = true;"));
    }

    #[test]
    fn html_exposes_window_action_shortcuts() {
        let document = html(&UiConfig::default());

        assert!(document.contains(
            r#"window.__RUNX_FOCUS_WINDOW_SHORTCUT__ = {"key":"Enter","code":null,"alt":false,"ctrl":false,"meta":false,"shift":false};"#
        ));
        assert!(document.contains(
            r#"window.__RUNX_ACTIVATE_ALL_WINDOWS_SHORTCUT__ = {"key":"Enter","code":null,"alt":true,"ctrl":false,"meta":false,"shift":false};"#
        ));
    }
}
