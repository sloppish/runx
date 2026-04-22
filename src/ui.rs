//! Embeds the static frontend templates into a themed HTML document.

use crate::config::UiConfig;

const HTML_TEMPLATE: &str = include_str!("../ui/index.html");
const STYLE_TEMPLATE: &str = include_str!("../ui/styles.css");
const SCRIPT_SOURCE: &str = include_str!("../ui/app.js");

/// Returns the full HTML document served into the embedded webview.
pub fn html(theme: &UiConfig) -> String {
    HTML_TEMPLATE
        .replace("__RUNX_STYLE__", &theme_css(theme))
        .replace("__RUNX_SCRIPT__", SCRIPT_SOURCE)
}

/// Returns the theme-expanded CSS used by the embedded webview.
pub fn theme_css(theme: &UiConfig) -> String {
    let label_font_size = format!("{}px", theme.font_sizes.label);
    let input_font_size = format!("{}px", theme.font_sizes.input);
    let title_font_size = format!("{}px", theme.font_sizes.title);
    let subtitle_font_size = format!("{}px", theme.font_sizes.subtitle);
    let badge_font_size = format!("{}px", theme.font_sizes.badge);
    let accelerator_font_size = format!("{}px", theme.font_sizes.accelerator);
    let config_error_title_font_size = format!("{}px", theme.font_sizes.config_error_title);
    let config_error_body_font_size = format!("{}px", theme.font_sizes.config_error_body);
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
    use super::theme_css;
    use crate::config::UiConfig;

    #[test]
    fn theme_css_includes_configured_font_sizes() {
        let mut theme = UiConfig::default();
        theme.font_sizes.input = 34;
        theme.font_sizes.title = 18;
        theme.font_sizes.config_error_body = 17;
        theme.layout.input_radius = 22;
        theme.layout.badge_size = 52;

        let css = theme_css(&theme);

        assert!(css.contains("--input-font-size: 34px;"));
        assert!(css.contains("--title-font-size: 18px;"));
        assert!(css.contains("--config-error-body-font-size: 17px;"));
        assert!(css.contains("--input-radius: 22px;"));
        assert!(css.contains("--badge-size: 52px;"));
    }
}
