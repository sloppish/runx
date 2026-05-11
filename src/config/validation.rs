use std::{collections::HashMap, path::Path};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use toml::{Spanned, Table};

use super::{
    defaults::{BUILTIN_COLORSCHEME_NAMES, KNOWN_PROVIDER_NAMES},
    errors::render_config_validation_error,
    schema::*,
    shortcuts::parse_hotkey_shortcut,
};

macro_rules! check_spanned {
    ($config_path:expr, $raw:expr, $field:expr, $validate:expr, $msg:expr) => {
        if let Some(field) = &$field
            && let Err(error) = $validate(*field.get_ref(), $msg)
        {
            return Err(anyhow!(render_config_validation_error(
                $config_path,
                $raw,
                &error.to_string(),
                field.span(),
            )));
        }
    };
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RawConfigSpans {
    hotkey: RawHotKeySpans,
    window: RawWindowSpans,
    display_overrides: Vec<RawDisplayOverrideSpans>,
    providers: RawProvidersSpans,
    ranking: RawRankingSpans,
    timing: TimingConfig,
    plugins: RawPluginsSpans,
    plugin: HashMap<String, Table>,
    ui: RawUiSpans,
    debug_log: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiSpans {
    show_header: bool,
    cycle_selection: bool,
    colorscheme: Option<Spanned<String>>,
    font_family: String,
    colorschemes: HashMap<String, RawUiColorschemeSpans>,
    canvas: RawUiCanvasSpans,
    entries: RawUiEntriesSpans,
    shortcuts: UiShortcutsConfig,
    font_sizes: UiFontSizesConfig,
    layout: UiLayoutConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiColorschemeSpans {
    base: Option<Spanned<String>>,
    #[serde(flatten)]
    _overrides: UiColorOverridesConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiCanvasSpans {
    show: bool,
    radius: u16,
    background_opacity: Option<Spanned<f64>>,
    #[serde(alias = "opacity")]
    chrome_opacity: Option<Spanned<f64>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiEntriesSpans {
    opacity: Option<Spanned<f64>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawHotKeySpans {
    shortcut: Option<Spanned<String>>,
    quick_switch: Option<Spanned<String>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawWindowSpans {
    width_fraction: Option<Spanned<f64>>,
    visible_rows: Option<Spanned<usize>>,
    min_width: Option<Spanned<f64>>,
    max_width: Option<Spanned<f64>>,
    min_height: Option<Spanned<f64>>,
    max_height: Option<Spanned<f64>>,
    hide_when_inactive: bool,
    always_on_top: bool,
    show_on: WindowDisplayTarget,
    scale: Option<Spanned<f64>>,
    show_animation: bool,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawDisplayOverrideSpans {
    built_in: Option<Spanned<bool>>,
    vendor: Option<Spanned<u32>>,
    model: Option<Spanned<u32>>,
    serial: Option<Spanned<u32>>,
    width_fraction: Option<Spanned<f64>>,
    visible_rows: Option<Spanned<usize>>,
    min_width: Option<Spanned<f64>>,
    max_width: Option<Spanned<f64>>,
    min_height: Option<Spanned<f64>>,
    max_height: Option<Spanned<f64>>,
    scale: Option<Spanned<f64>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawPluginsSpans {
    directories: Vec<String>,
    search_paths: Vec<String>,
    install: Vec<RawPluginInstallSpans>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct RawPluginInstallSpans {
    source: Spanned<String>,
    name: Option<String>,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
    branch: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawProvidersSpans {
    disabled: Vec<Spanned<String>>,
    windows: WindowsProviderConfig,
    apps: AppsProviderConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawRankingSpans {
    tie_threshold: i64,
    provider_order: Vec<Spanned<String>>,
    provider_score_boosts: RawProviderScoreBoostsSpans,
    score_rules: Vec<RawRankingScoreRuleSpans>,
    result_limit: usize,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawProviderScoreBoostsSpans {
    windows: Option<i64>,
    apps: Option<i64>,
    settings: Option<i64>,
    plugins: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawRankingScoreRuleSpans {
    providers: Vec<Spanned<String>>,
    field: RankingScoreRuleField,
    #[serde(rename = "match")]
    match_kind: RankingScoreRuleMatchKind,
    pattern: String,
    boost: i64,
}

pub(super) fn validate_config(config: &Config) -> Result<()> {
    validate_plugin_install_entries(&config.plugins.install)?;
    validate_provider_names(&config.providers.disabled, "[providers].disabled")?;
    validate_provider_names(&config.ranking.provider_order, "[ranking].provider_order")?;
    validate_window_dimension_fraction(
        config.window.width_fraction,
        "[window].width_fraction must be greater than 0.0 and at most 1.0",
    )?;
    validate_visible_rows(
        config.window.visible_rows,
        "[window].visible_rows must be greater than 0",
    )?;
    validate_dimension_clamps(
        config.window.min_width,
        config.window.max_width,
        config.window.min_height,
        config.window.max_height,
        "[window]",
    )?;
    for (index, display_override) in config.display_overrides.iter().enumerate() {
        validate_display_override(
            display_override,
            &format!("[[display_overrides]] entry {}", index + 1),
        )?;
    }
    validate_positive_scale(
        config.window.scale,
        "[window].scale must be greater than 0.0",
    )?;
    validate_opacity(
        config.ui.canvas.background_opacity,
        "[ui.canvas].background_opacity must be between 0.0 and 1.0",
    )?;
    validate_opacity(
        config.ui.canvas.chrome_opacity,
        "[ui.canvas].chrome_opacity must be between 0.0 and 1.0",
    )?;
    validate_opacity(
        config.ui.entries.opacity,
        "[ui.entries].opacity must be between 0.0 and 1.0",
    )?;
    validate_ui_colorscheme_selector(config)?;
    validate_ui_colorschemes(&config.ui.colorschemes)?;

    for provider in config.ranking.provider_score_boosts.keys() {
        validate_provider_name(provider, "[ranking.provider_score_boosts] key")?;
    }

    for (index, rule) in config.ranking.score_rules.iter().enumerate() {
        let context = format!("[[ranking.score_rules]] entry {}", index + 1);
        validate_provider_names(&rule.providers, &format!("{context}.providers"))?;
    }

    Ok(())
}

pub(super) fn validate_config_with_spans(
    config_path: &Path,
    raw: &str,
    spans: &RawConfigSpans,
) -> Result<()> {
    if let Some(shortcut) = &spans.hotkey.shortcut
        && !is_disabled_shortcut_value(shortcut.get_ref())
        && let Err(error) = parse_hotkey_shortcut(shortcut.get_ref())
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            shortcut.span(),
        )));
    }

    if let Some(qs) = &spans.hotkey.quick_switch
        && !is_disabled_shortcut_value(qs.get_ref())
        && let Err(error) = parse_hotkey_shortcut(qs.get_ref())
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            qs.span(),
        )));
    }

    validate_spanned_provider_names(
        config_path,
        raw,
        &spans.providers.disabled,
        "[providers].disabled",
    )?;
    validate_spanned_provider_names(
        config_path,
        raw,
        &spans.ranking.provider_order,
        "[ranking].provider_order",
    )?;

    check_spanned!(
        config_path,
        raw,
        spans.window.width_fraction,
        validate_window_dimension_fraction,
        "[window].width_fraction must be greater than 0.0 and at most 1.0"
    );
    check_spanned!(
        config_path,
        raw,
        spans.window.visible_rows,
        validate_visible_rows,
        "[window].visible_rows must be greater than 0"
    );
    check_spanned!(
        config_path,
        raw,
        spans.window.min_width,
        validate_positive_dimension,
        "[window].min_width must be greater than 0.0"
    );
    check_spanned!(
        config_path,
        raw,
        spans.window.max_width,
        validate_positive_dimension,
        "[window].max_width must be greater than 0.0"
    );
    check_spanned!(
        config_path,
        raw,
        spans.window.min_height,
        validate_positive_dimension,
        "[window].min_height must be greater than 0.0"
    );
    check_spanned!(
        config_path,
        raw,
        spans.window.max_height,
        validate_positive_dimension,
        "[window].max_height must be greater than 0.0"
    );

    if let (Some(min_width), Some(max_width)) = (&spans.window.min_width, &spans.window.max_width)
        && let Err(error) = validate_dimension_range(
            *min_width.get_ref(),
            *max_width.get_ref(),
            "[window].min_width must be less than or equal to [window].max_width",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            max_width.span(),
        )));
    }

    if let (Some(min_height), Some(max_height)) =
        (&spans.window.min_height, &spans.window.max_height)
        && let Err(error) = validate_dimension_range(
            *min_height.get_ref(),
            *max_height.get_ref(),
            "[window].min_height must be less than or equal to [window].max_height",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            max_height.span(),
        )));
    }

    for (index, display_override) in spans.display_overrides.iter().enumerate() {
        let ctx = format!("[[display_overrides]] entry {}", index + 1);

        check_spanned!(
            config_path,
            raw,
            display_override.serial,
            validate_positive_display_identifier,
            &format!("{ctx}.serial must be greater than 0")
        );
        check_spanned!(
            config_path,
            raw,
            display_override.vendor,
            validate_positive_display_identifier,
            &format!("{ctx}.vendor must be greater than 0")
        );
        check_spanned!(
            config_path,
            raw,
            display_override.model,
            validate_positive_display_identifier,
            &format!("{ctx}.model must be greater than 0")
        );

        if display_override.vendor.is_some() != display_override.model.is_some() {
            let span = display_override
                .vendor
                .as_ref()
                .map(Spanned::span)
                .or_else(|| display_override.model.as_ref().map(Spanned::span))
                .unwrap_or(0..0);
            let message = format!("{ctx} must set vendor and model together");
            return Err(anyhow!(render_config_validation_error(
                config_path,
                raw,
                &message,
                span,
            )));
        }

        check_spanned!(
            config_path,
            raw,
            display_override.width_fraction,
            validate_window_dimension_fraction,
            &format!("{ctx}.width_fraction must be greater than 0.0 and at most 1.0")
        );
        check_spanned!(
            config_path,
            raw,
            display_override.visible_rows,
            validate_visible_rows,
            &format!("{ctx}.visible_rows must be greater than 0")
        );
        check_spanned!(
            config_path,
            raw,
            display_override.min_width,
            validate_positive_dimension,
            &format!("{ctx}.min_width must be greater than 0.0")
        );
        check_spanned!(
            config_path,
            raw,
            display_override.max_width,
            validate_positive_dimension,
            &format!("{ctx}.max_width must be greater than 0.0")
        );
        check_spanned!(
            config_path,
            raw,
            display_override.min_height,
            validate_positive_dimension,
            &format!("{ctx}.min_height must be greater than 0.0")
        );
        check_spanned!(
            config_path,
            raw,
            display_override.max_height,
            validate_positive_dimension,
            &format!("{ctx}.max_height must be greater than 0.0")
        );

        if let (Some(min_width), Some(max_width)) =
            (&display_override.min_width, &display_override.max_width)
            && let Err(error) = validate_dimension_range(
                *min_width.get_ref(),
                *max_width.get_ref(),
                &format!("{ctx}.min_width must be less than or equal to {ctx}.max_width"),
            )
        {
            return Err(anyhow!(render_config_validation_error(
                config_path,
                raw,
                &error.to_string(),
                max_width.span(),
            )));
        }

        if let (Some(min_height), Some(max_height)) =
            (&display_override.min_height, &display_override.max_height)
            && let Err(error) = validate_dimension_range(
                *min_height.get_ref(),
                *max_height.get_ref(),
                &format!("{ctx}.min_height must be less than or equal to {ctx}.max_height"),
            )
        {
            return Err(anyhow!(render_config_validation_error(
                config_path,
                raw,
                &error.to_string(),
                max_height.span(),
            )));
        }

        check_spanned!(
            config_path,
            raw,
            display_override.scale,
            validate_positive_scale,
            &format!("{ctx}.scale must be greater than 0.0")
        );
    }

    for (index, rule) in spans.ranking.score_rules.iter().enumerate() {
        validate_spanned_provider_names(
            config_path,
            raw,
            &rule.providers,
            &format!("[[ranking.score_rules]] entry {}.providers", index + 1),
        )?;
    }

    check_spanned!(
        config_path,
        raw,
        spans.window.scale,
        validate_positive_scale,
        "[window].scale must be greater than 0.0"
    );
    check_spanned!(
        config_path,
        raw,
        spans.ui.canvas.background_opacity,
        validate_opacity,
        "[ui.canvas].background_opacity must be between 0.0 and 1.0"
    );
    check_spanned!(
        config_path,
        raw,
        spans.ui.canvas.chrome_opacity,
        validate_opacity,
        "[ui.canvas].chrome_opacity must be between 0.0 and 1.0"
    );
    check_spanned!(
        config_path,
        raw,
        spans.ui.entries.opacity,
        validate_opacity,
        "[ui.entries].opacity must be between 0.0 and 1.0"
    );

    if let Some(colorscheme) = &spans.ui.colorscheme
        && let Err(error) = validate_ui_colorscheme_name_impl(
            colorscheme.get_ref(),
            spans.ui.colorschemes.keys().map(String::as_str),
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            colorscheme.span(),
        )));
    }

    for (name, scheme) in &spans.ui.colorschemes {
        if BUILTIN_COLORSCHEME_NAMES.contains(&name.as_str()) {
            let message = readonly_builtin_colorscheme_message(name);
            let span = scheme
                .base
                .as_ref()
                .map(|base| base.span())
                .or_else(|| colorscheme_table_header_span(raw, name))
                .unwrap_or(0..0);
            return Err(anyhow!(render_config_validation_error(
                config_path,
                raw,
                &message,
                span,
            )));
        }

        if let Some(base) = &scheme.base
            && !BUILTIN_COLORSCHEME_NAMES.contains(&base.get_ref().as_str())
        {
            let message = format!(
                "[ui.colorschemes.{name}].base must be one of: {}",
                BUILTIN_COLORSCHEME_NAMES.join(", ")
            );
            return Err(anyhow!(render_config_validation_error(
                config_path,
                raw,
                &message,
                base.span(),
            )));
        }
    }

    Ok(())
}

fn validate_opacity(opacity: f64, context: &str) -> Result<()> {
    if (0.0..=1.0).contains(&opacity) {
        Ok(())
    } else {
        bail!("{context}");
    }
}

fn validate_window_dimension_fraction(value: f64, context: &str) -> Result<()> {
    if value.is_finite() && value > 0.0 && value <= 1.0 {
        Ok(())
    } else {
        bail!("{context}");
    }
}

fn validate_visible_rows(value: usize, context: &str) -> Result<()> {
    if value > 0 {
        Ok(())
    } else {
        bail!("{context}");
    }
}

fn validate_positive_dimension(value: f64, context: &str) -> Result<()> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        bail!("{context}");
    }
}

fn validate_dimension_range(min: f64, max: f64, context: &str) -> Result<()> {
    if min <= max {
        Ok(())
    } else {
        bail!("{context}");
    }
}

fn validate_dimension_clamps(
    min_width: Option<f64>,
    max_width: Option<f64>,
    min_height: Option<f64>,
    max_height: Option<f64>,
    context: &str,
) -> Result<()> {
    if let Some(value) = min_width {
        validate_positive_dimension(
            value,
            &format!("{context}.min_width must be greater than 0.0"),
        )?;
    }
    if let Some(value) = max_width {
        validate_positive_dimension(
            value,
            &format!("{context}.max_width must be greater than 0.0"),
        )?;
    }
    if let Some(value) = min_height {
        validate_positive_dimension(
            value,
            &format!("{context}.min_height must be greater than 0.0"),
        )?;
    }
    if let Some(value) = max_height {
        validate_positive_dimension(
            value,
            &format!("{context}.max_height must be greater than 0.0"),
        )?;
    }
    if let (Some(min), Some(max)) = (min_width, max_width) {
        validate_dimension_range(
            min,
            max,
            &format!("{context}.min_width must be less than or equal to {context}.max_width"),
        )?;
    }
    if let (Some(min), Some(max)) = (min_height, max_height) {
        validate_dimension_range(
            min,
            max,
            &format!("{context}.min_height must be less than or equal to {context}.max_height"),
        )?;
    }
    Ok(())
}

fn validate_positive_scale(value: f64, context: &str) -> Result<()> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        bail!("{context}");
    }
}

fn validate_positive_display_identifier(value: u32, context: &str) -> Result<()> {
    if value > 0 {
        Ok(())
    } else {
        bail!("{context}");
    }
}

fn validate_provider_names(providers: &[String], context: &str) -> Result<()> {
    for provider in providers {
        validate_provider_name(provider, context)?;
    }
    Ok(())
}

fn validate_provider_name(provider: &str, context: &str) -> Result<()> {
    if KNOWN_PROVIDER_NAMES.contains(&provider) {
        Ok(())
    } else {
        bail!(
            "{context} contains unknown provider `{provider}`. Expected one of: {}",
            KNOWN_PROVIDER_NAMES.join(", ")
        )
    }
}

fn validate_spanned_provider_names(
    config_path: &Path,
    raw: &str,
    providers: &[Spanned<String>],
    context: &str,
) -> Result<()> {
    for provider in providers {
        let name = provider.get_ref();
        if !KNOWN_PROVIDER_NAMES.contains(&name.as_str()) {
            let message = format!(
                "{context} contains unknown provider `{name}`. Expected one of: {}",
                KNOWN_PROVIDER_NAMES.join(", ")
            );
            return Err(anyhow!(render_config_validation_error(
                config_path,
                raw,
                &message,
                provider.span(),
            )));
        }
    }

    Ok(())
}

fn validate_display_override(config: &DisplayOverrideConfig, context: &str) -> Result<()> {
    let matcher_count = config.built_in.is_some() as usize
        + config.vendor.is_some() as usize
        + config.model.is_some() as usize
        + config.serial.is_some() as usize;
    if matcher_count == 0 {
        bail!("{context} must match at least one display attribute");
    }

    if config.vendor.is_some() != config.model.is_some() {
        bail!("{context} must set vendor and model together");
    }

    if let Some(serial) = config.serial {
        validate_positive_display_identifier(
            serial,
            &format!("{context}.serial must be greater than 0"),
        )?;
    }
    if let Some(vendor) = config.vendor {
        validate_positive_display_identifier(
            vendor,
            &format!("{context}.vendor must be greater than 0"),
        )?;
    }
    if let Some(model) = config.model {
        validate_positive_display_identifier(
            model,
            &format!("{context}.model must be greater than 0"),
        )?;
    }

    let has_override = config.width_fraction.is_some()
        || config.visible_rows.is_some()
        || config.min_width.is_some()
        || config.max_width.is_some()
        || config.min_height.is_some()
        || config.max_height.is_some()
        || config.scale.is_some();
    if !has_override {
        bail!("{context} must override at least one setting");
    }

    if let Some(value) = config.width_fraction {
        validate_window_dimension_fraction(
            value,
            &format!("{context}.width_fraction must be greater than 0.0 and at most 1.0"),
        )?;
    }
    if let Some(value) = config.visible_rows {
        validate_visible_rows(
            value,
            &format!("{context}.visible_rows must be greater than 0"),
        )?;
    }
    validate_dimension_clamps(
        config.min_width,
        config.max_width,
        config.min_height,
        config.max_height,
        context,
    )?;
    if let Some(value) = config.scale {
        validate_positive_scale(value, &format!("{context}.scale must be greater than 0.0"))?;
    }

    Ok(())
}

fn validate_ui_colorscheme_selector(config: &Config) -> Result<()> {
    validate_ui_colorscheme_name_impl(
        &config.ui.colorscheme,
        config.ui.colorschemes.keys().map(String::as_str),
    )
}

fn validate_ui_colorschemes(colorschemes: &HashMap<String, UiColorschemeConfig>) -> Result<()> {
    for (name, scheme) in colorschemes {
        if BUILTIN_COLORSCHEME_NAMES.contains(&name.as_str()) {
            bail!("{}", readonly_builtin_colorscheme_message(name));
        }

        if let Some(base) = &scheme.base
            && !BUILTIN_COLORSCHEME_NAMES.contains(&base.as_str())
        {
            bail!(
                "[ui.colorschemes.{name}].base must be one of: {}",
                BUILTIN_COLORSCHEME_NAMES.join(", ")
            );
        }

        if scheme.base.is_none() {
            let missing = scheme.overrides.missing_required_fields();
            if !missing.is_empty() {
                bail!(
                    "[ui.colorschemes.{name}] has no base, so it must define every color token. Missing: {}",
                    missing.join(", ")
                );
            }
        }
    }
    Ok(())
}

fn readonly_builtin_colorscheme_message(name: &str) -> String {
    format!(
        "[ui.colorschemes.{name}] is read-only; create a custom colorscheme with base = \"{name}\" to override it"
    )
}

fn colorscheme_table_header_span(raw: &str, name: &str) -> Option<std::ops::Range<usize>> {
    let header = format!("[ui.colorschemes.{name}]");
    raw.find(&header).map(|start| start..start + header.len())
}

fn validate_plugin_install_entries(entries: &[PluginInstallEntry]) -> Result<()> {
    for (index, entry) in entries.iter().enumerate() {
        let context = format!("[[plugins.install]] entry {}", index + 1);
        if entry.source.trim().is_empty() {
            bail!("{context}.source must not be empty");
        }
        if entry.git_ref.is_some() && entry.branch.is_some() {
            bail!("{context} cannot set both `ref` and `branch`");
        }
        if let Some(git_ref) = &entry.git_ref
            && git_ref.trim().is_empty()
        {
            bail!("{context}.ref must not be empty");
        }
        if let Some(branch) = &entry.branch
            && branch.trim().is_empty()
        {
            bail!("{context}.branch must not be empty");
        }
    }

    let mut seen_names = std::collections::HashSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let resolved = entry.name.as_deref().unwrap_or_else(|| {
            entry
                .source
                .trim_end_matches('/')
                .trim_end_matches(".git")
                .rsplit('/')
                .next()
                .unwrap_or("plugin")
        });
        if !seen_names.insert(resolved) {
            bail!(
                "[[plugins.install]] entry {} has a duplicate install name `{resolved}`",
                index + 1,
            );
        }
    }

    Ok(())
}

fn validate_ui_colorscheme_name_impl<'a>(
    name: &str,
    available: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    if name == "system" {
        return Ok(());
    }

    if BUILTIN_COLORSCHEME_NAMES.contains(&name) {
        return Ok(());
    }

    if available.into_iter().any(|candidate| candidate == name) {
        Ok(())
    } else {
        bail!(
            "[ui].colorscheme must be `system`, one of the built-ins (`{}`), or a custom name under [ui.colorschemes.<name>]",
            BUILTIN_COLORSCHEME_NAMES.join("`, `")
        )
    }
}

fn is_disabled_shortcut_value(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none")
}
