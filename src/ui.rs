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

fn theme_css(theme: &UiConfig) -> String {
    let replacements = [
        ("__ACCENT__", theme.accent.as_str()),
        ("__BACKGROUND__", theme.background.as_str()),
        ("__PANEL__", theme.panel.as_str()),
        ("__TEXT__", theme.text.as_str()),
        ("__MUTED__", theme.muted.as_str()),
        ("__FONT_FAMILY__", theme.font_family.as_str()),
    ];

    let mut css = STYLE_TEMPLATE.to_owned();
    for (token, value) in replacements {
        css = css.replace(token, value);
    }
    css
}
