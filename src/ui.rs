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

        let css = theme_css(&theme);

        assert!(css.contains("--input-font-size: 34px;"));
        assert!(css.contains("--title-font-size: 18px;"));
        assert!(css.contains("--config-error-body-font-size: 17px;"));
    }
}
