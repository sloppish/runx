use std::{collections::HashMap, path::Path};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use toml::{Spanned, Table};

use super::{
    defaults::{BUILTIN_COLORSCHEME_NAMES, KNOWN_PROVIDER_NAMES},
    errors::render_config_validation_error,
    schema::*,
    shortcuts::{parse_key, parse_modifier},
};

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RawConfigSpans {
    hotkey: RawHotKeySpans,
    window: RawWindowSpans,
    providers: RawProvidersSpans,
    ranking: RawRankingSpans,
    timing: TimingConfig,
    plugins: PluginsConfig,
    plugin: HashMap<String, Table>,
    ui: RawUiSpans,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiSpans {
    show_header: bool,
    cycle_selection: bool,
    colorscheme: Option<Spanned<String>>,
    font_family: String,
    scale: Option<Spanned<f64>>,
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
    #[allow(dead_code)]
    #[serde(flatten)]
    overrides: UiColorOverridesConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiCanvasSpans {
    show: bool,
    radius: u16,
    opacity: Option<Spanned<f64>>,
    background_opacity: Option<Spanned<f64>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawUiEntriesSpans {
    opacity: Option<Spanned<f64>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawHotKeySpans {
    key: Option<Spanned<String>>,
    modifiers: Vec<Spanned<String>>,
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
    hide_on_blur: bool,
    always_on_top: bool,
    show_on: WindowDisplayTarget,
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
    empty_query_providers: Vec<Spanned<String>>,
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
    validate_provider_names(&config.providers.disabled, "[providers].disabled")?;
    validate_provider_names(&config.ranking.provider_order, "[ranking].provider_order")?;
    validate_provider_names(
        &config.ranking.empty_query_providers,
        "[ranking].empty_query_providers",
    )?;
    validate_window_dimension_fraction(
        config.window.width_fraction,
        "[window].width_fraction must be greater than 0.0 and at most 1.0",
    )?;
    validate_visible_rows(
        config.window.visible_rows,
        "[window].visible_rows must be greater than 0",
    )?;
    validate_positive_dimension(
        config.window.min_width,
        "[window].min_width must be greater than 0.0",
    )?;
    validate_positive_dimension(
        config.window.max_width,
        "[window].max_width must be greater than 0.0",
    )?;
    validate_positive_dimension(
        config.window.min_height,
        "[window].min_height must be greater than 0.0",
    )?;
    validate_positive_dimension(
        config.window.max_height,
        "[window].max_height must be greater than 0.0",
    )?;
    validate_dimension_range(
        config.window.min_width,
        config.window.max_width,
        "[window].min_width must be less than or equal to [window].max_width",
    )?;
    validate_dimension_range(
        config.window.min_height,
        config.window.max_height,
        "[window].min_height must be less than or equal to [window].max_height",
    )?;
    validate_positive_scale(config.ui.scale, "[ui].scale must be greater than 0.0")?;
    validate_opacity(
        config.ui.canvas.opacity,
        "[ui.canvas].opacity must be between 0.0 and 1.0",
    )?;
    validate_opacity(
        config.ui.canvas.background_opacity,
        "[ui.canvas].background_opacity must be between 0.0 and 1.0",
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
    if let Some(key) = &spans.hotkey.key
        && let Err(error) = parse_key(key.get_ref())
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            key.span(),
        )));
    }

    for modifier in &spans.hotkey.modifiers {
        if let Err(error) = parse_modifier(modifier.get_ref()) {
            return Err(anyhow!(render_config_validation_error(
                config_path,
                raw,
                &error.to_string(),
                modifier.span(),
            )));
        }
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
    validate_spanned_provider_names(
        config_path,
        raw,
        &spans.ranking.empty_query_providers,
        "[ranking].empty_query_providers",
    )?;

    if let Some(width_fraction) = &spans.window.width_fraction
        && let Err(error) = validate_window_dimension_fraction(
            *width_fraction.get_ref(),
            "[window].width_fraction must be greater than 0.0 and at most 1.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            width_fraction.span(),
        )));
    }

    if let Some(visible_rows) = &spans.window.visible_rows
        && let Err(error) = validate_visible_rows(
            *visible_rows.get_ref(),
            "[window].visible_rows must be greater than 0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            visible_rows.span(),
        )));
    }

    if let Some(min_width) = &spans.window.min_width
        && let Err(error) = validate_positive_dimension(
            *min_width.get_ref(),
            "[window].min_width must be greater than 0.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            min_width.span(),
        )));
    }

    if let Some(max_width) = &spans.window.max_width
        && let Err(error) = validate_positive_dimension(
            *max_width.get_ref(),
            "[window].max_width must be greater than 0.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            max_width.span(),
        )));
    }

    if let Some(min_height) = &spans.window.min_height
        && let Err(error) = validate_positive_dimension(
            *min_height.get_ref(),
            "[window].min_height must be greater than 0.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            min_height.span(),
        )));
    }

    if let Some(max_height) = &spans.window.max_height
        && let Err(error) = validate_positive_dimension(
            *max_height.get_ref(),
            "[window].max_height must be greater than 0.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            max_height.span(),
        )));
    }

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

    for (index, rule) in spans.ranking.score_rules.iter().enumerate() {
        validate_spanned_provider_names(
            config_path,
            raw,
            &rule.providers,
            &format!("[[ranking.score_rules]] entry {}.providers", index + 1),
        )?;
    }

    if let Some(scale) = &spans.ui.scale
        && let Err(error) =
            validate_positive_scale(*scale.get_ref(), "[ui].scale must be greater than 0.0")
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            scale.span(),
        )));
    }

    if let Some(opacity) = &spans.ui.canvas.opacity
        && let Err(error) = validate_opacity(
            *opacity.get_ref(),
            "[ui.canvas].opacity must be between 0.0 and 1.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            opacity.span(),
        )));
    }

    if let Some(opacity) = &spans.ui.canvas.background_opacity
        && let Err(error) = validate_opacity(
            *opacity.get_ref(),
            "[ui.canvas].background_opacity must be between 0.0 and 1.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            opacity.span(),
        )));
    }

    if let Some(opacity) = &spans.ui.entries.opacity
        && let Err(error) = validate_opacity(
            *opacity.get_ref(),
            "[ui.entries].opacity must be between 0.0 and 1.0",
        )
    {
        return Err(anyhow!(render_config_validation_error(
            config_path,
            raw,
            &error.to_string(),
            opacity.span(),
        )));
    }

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
            if let Some(base) = &scheme.base {
                let message = format!(
                    "[ui.colorschemes.{name}].base is not allowed for built-in colorschemes"
                );
                return Err(anyhow!(render_config_validation_error(
                    config_path,
                    raw,
                    &message,
                    base.span(),
                )));
            }
            continue;
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

fn validate_positive_scale(value: f64, context: &str) -> Result<()> {
    if value.is_finite() && value > 0.0 {
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

fn validate_ui_colorscheme_selector(config: &Config) -> Result<()> {
    validate_ui_colorscheme_name_impl(
        &config.ui.colorscheme,
        config.ui.colorschemes.keys().map(String::as_str),
    )
}

fn validate_ui_colorschemes(colorschemes: &HashMap<String, UiColorschemeConfig>) -> Result<()> {
    for (name, scheme) in colorschemes {
        if BUILTIN_COLORSCHEME_NAMES.contains(&name.as_str()) {
            if scheme.base.is_some() {
                bail!("[ui.colorschemes.{name}].base is not allowed for built-in colorschemes");
            }
            continue;
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
