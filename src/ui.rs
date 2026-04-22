//! Embeds the static frontend templates into a themed HTML document.

use crate::config::UiConfig;

const HTML_TEMPLATE: &str = include_str!("../ui/index.html");
const STYLE_TEMPLATE: &str = include_str!("../ui/styles.css");
const SCRIPT_SOURCE: &str = include_str!("../ui/app.js");

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
        .replace("__RUNX_STYLE__", &theme_css(theme))
        .replace("__RUNX_SCRIPT__", SCRIPT_SOURCE)
}

/// Returns the theme-expanded CSS used by the embedded webview.
pub fn theme_css(theme: &UiConfig) -> String {
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
        ("__ACCENT__", theme.accent.as_str()),
        ("__BACKGROUND__", theme.background.as_str()),
        ("__PANEL__", theme.panel.as_str()),
        ("__TEXT__", theme.text.as_str()),
        ("__MUTED__", theme.muted.as_str()),
        ("__FONT_FAMILY__", theme.font_family.as_str()),
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
    use crate::config::UiConfig;

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
        assert!(css.contains("--input-font-size: 34px;"));
        assert!(css.contains("--title-font-size: 18px;"));
        assert!(css.contains("--config-error-body-font-size: 17px;"));
        assert!(css.contains("--section-gap: 16px;"));
        assert!(css.contains("--input-radius: 22px;"));
        assert!(css.contains("--badge-size: 52px;"));
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
}
