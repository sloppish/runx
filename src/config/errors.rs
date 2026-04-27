use std::path::Path;

use miette::{
    GraphicalReportHandler, GraphicalTheme, LabeledSpan, MietteDiagnostic, NamedSource, Report,
};

pub(super) fn render_toml_parse_error(
    config_path: &Path,
    raw: &str,
    error: &toml::de::Error,
) -> String {
    render_config_error_report(
        config_path,
        raw,
        "failed to parse",
        "runx::config::toml_parse",
        error.message(),
        error.span(),
    )
}

pub(super) fn render_config_validation_error(
    config_path: &Path,
    raw: &str,
    message: &str,
    span: std::ops::Range<usize>,
) -> String {
    render_config_error_report(
        config_path,
        raw,
        "invalid configuration",
        "runx::config::validation",
        message,
        Some(span),
    )
}

fn render_config_error_report(
    config_path: &Path,
    raw: &str,
    title: &str,
    code: &str,
    message: &str,
    span: Option<std::ops::Range<usize>>,
) -> String {
    let mut diagnostic = MietteDiagnostic::new(format!("{title} {}", config_path.display()))
        .with_code(code)
        .with_help("Fix the highlighted config value, then save config.toml again.");

    if let Some(span) = span {
        diagnostic =
            diagnostic.with_label(LabeledSpan::new_with_span(Some(message.to_owned()), span));
    }

    let source = NamedSource::new(config_path.display().to_string(), raw.to_owned());
    let report = Report::new(diagnostic).with_source_code(source);
    let handler = GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor())
        .without_cause_chain()
        .with_context_lines(2)
        .with_width(100);
    let mut rendered = String::new();

    if handler
        .render_report(&mut rendered, report.as_ref())
        .is_ok()
    {
        rendered
    } else {
        format!("{title} {}: {message}", config_path.display())
    }
}
