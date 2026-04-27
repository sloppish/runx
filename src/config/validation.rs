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
    window: WindowConfig,
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

    for (index, rule) in spans.ranking.score_rules.iter().enumerate() {
        validate_spanned_provider_names(
            config_path,
            raw,
            &rule.providers,
            &format!("[[ranking.score_rules]] entry {}.providers", index + 1),
        )?;
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
