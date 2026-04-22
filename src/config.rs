//! Loading and normalizing `config.toml`.
//!
//! The config layer owns both user-facing deserialization and the derived
//! runtime values that the rest of the launcher needs, such as resolved plugin
//! directories, command routes, and search paths.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result, anyhow, bail};
use directories::BaseDirs;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use miette::{
    GraphicalReportHandler, GraphicalTheme, LabeledSpan, MietteDiagnostic, NamedSource, Report,
};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use toml::{Spanned, Table};

const DEFAULT_CONFIG: &str = r##"# Runx configuration
#
# `provider_order` controls which provider wins when scores are close.
# `tie_threshold` is the raw fuzzy-score delta that still counts as "similar".
# `empty_query_providers` controls which providers run before you type anything.
# `provider_score_boosts` lets you nudge merged scores per provider.
# `score_rules` lets you boost or demote specific result text patterns.
# `search_debounce_ms` and `render_coalesce_ms` tune search/render scheduling.
# `focus_behavior` controls how Runx tries to surface a selected window result.

[hotkey]
key = "Space"
modifiers = ["Alt"]

[window]
width = 760
height = 520
hide_on_blur = true
always_on_top = true
show_on = "primary"
focus_behavior = "focus_window_then_activate_fallback"

[ranking]
tie_threshold = 120
provider_order = ["windows", "apps", "settings", "plugins", "spotlight"]
empty_query_providers = ["windows"]
result_limit = 24
# Example:
# [ranking.provider_score_boosts]
# spotlight = -150
# [[ranking.score_rules]]
# providers = ["apps", "windows"]
# field = "title"
# match = "contains"
# pattern = "spotify"
# boost = 120

[timing]
search_debounce_ms = 24
render_coalesce_ms = 8

[plugins]
directories = []
search_paths = []
# Example:
# search_paths = ["/opt/homebrew/bin"]

# Per-plugin configuration can live under `[plugin.<id>]`.
# Command routing can be configured under `[plugin.<id>.commands]`.
[ui]
font_family = "\"SF Pro Display\", \"Avenir Next\", \"Helvetica Neue\", sans-serif"
accent = "#c77b49"
background = "#f3ede5"
panel = "#fffaf3"
text = "#1f1a16"
muted = "#756759"

[ui.font_sizes]
label = 10
input = 30
title = 16
subtitle = 12
badge = 11
accelerator = 12
config_error_title = 24
config_error_body = 15
"##;

const KNOWN_PROVIDER_NAMES: [&str; 5] = ["windows", "apps", "settings", "plugins", "spotlight"];

/// Fully loaded configuration together with derived filesystem paths.
pub struct LoadedConfig {
    pub config: Config,
    pub config_path: PathBuf,
    pub plugin_dirs: Vec<PathBuf>,
    pub plugin_search_paths: Vec<PathBuf>,
}

/// Root configuration object deserialized from `config.toml`.
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub hotkey: HotKeyConfig,
    pub window: WindowConfig,
    pub ranking: RankingConfig,
    pub timing: TimingConfig,
    pub plugins: PluginsConfig,
    pub plugin: HashMap<String, Table>,
    pub ui: UiConfig,
}

/// User-facing global hotkey configuration.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HotKeyConfig {
    pub key: String,
    pub modifiers: Vec<String>,
}

/// Launcher window behavior and geometry.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct WindowConfig {
    pub width: f64,
    pub height: f64,
    pub hide_on_blur: bool,
    pub always_on_top: bool,
    pub show_on: WindowDisplayTarget,
    pub focus_behavior: WindowFocusBehavior,
}

/// Monitor selection strategy for placing the launcher window.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowDisplayTarget {
    #[default]
    Primary,
    Cursor,
}

/// Strategy for focusing a selected window result.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowFocusBehavior {
    ActivateAppOnly,
    ActivateAppThenFocusWindow,
    FocusWindowOnly,
    #[default]
    FocusWindowThenActivateFallback,
}

/// Ranking and truncation rules for the merged result list.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RankingConfig {
    pub tie_threshold: i64,
    pub provider_order: Vec<String>,
    pub empty_query_providers: Vec<String>,
    pub provider_score_boosts: HashMap<String, i64>,
    pub score_rules: Vec<RankingScoreRule>,
    pub result_limit: usize,
}

/// One additive ranking rule matched against a result row.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RankingScoreRule {
    pub providers: Vec<String>,
    pub field: RankingScoreRuleField,
    #[serde(rename = "match")]
    pub match_kind: RankingScoreRuleMatchKind,
    pub pattern: String,
    pub boost: i64,
}

/// Search-item field targeted by a ranking score rule.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RankingScoreRuleField {
    #[default]
    Title,
    Subtitle,
    Badge,
    Id,
}

/// Text-matching mode used by a ranking score rule.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RankingScoreRuleMatchKind {
    Exact,
    Prefix,
    #[default]
    Contains,
}

/// Debounce and coalescing timings for the search/render pipeline.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TimingConfig {
    pub search_debounce_ms: u64,
    pub render_coalesce_ms: u64,
}

/// Plugin discovery and subprocess lookup configuration.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PluginsConfig {
    pub directories: Vec<String>,
    pub search_paths: Vec<String>,
}

/// Theme tokens injected into the embedded HTML/CSS UI templates.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub font_family: String,
    pub accent: String,
    pub background: String,
    pub panel: String,
    pub text: String,
    pub muted: String,
    pub font_sizes: UiFontSizesConfig,
}

/// Font-size tokens injected into the embedded UI theme.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UiFontSizesConfig {
    pub label: u16,
    pub input: u16,
    pub title: u16,
    pub subtitle: u16,
    pub badge: u16,
    pub accelerator: u16,
    pub config_error_title: u16,
    pub config_error_body: u16,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawConfigSpans {
    hotkey: RawHotKeySpans,
    window: WindowConfig,
    ranking: RawRankingSpans,
    timing: TimingConfig,
    plugins: PluginsConfig,
    plugin: HashMap<String, Table>,
    ui: UiConfig,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawHotKeySpans {
    key: Option<Spanned<String>>,
    modifiers: Vec<Spanned<String>>,
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
    spotlight: Option<i64>,
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

impl LoadedConfig {
    /// Loads the user config, writing the default template on first launch.
    pub fn load() -> Result<Self> {
        let (root_dir, plugin_dir, config_path) = runtime_paths()?;
        if !config_path.exists() {
            fs::write(&config_path, DEFAULT_CONFIG)
                .with_context(|| format!("failed to write {}", config_path.display()))?;
        }

        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let config: Config = toml::from_str(&raw)
            .map_err(|error| anyhow!(render_toml_parse_error(&config_path, &raw, &error)))?;
        let raw_spans: RawConfigSpans = toml::from_str(&raw)
            .map_err(|error| anyhow!(render_toml_parse_error(&config_path, &raw, &error)))?;
        validate_config_with_spans(&config_path, &raw, &raw_spans)?;

        Self::from_parts(config, root_dir, plugin_dir, config_path)
    }

    /// Builds a runtime config from built-in defaults without persisting anything.
    pub fn load_defaults() -> Result<Self> {
        let (root_dir, plugin_dir, config_path) = runtime_paths()?;
        Self::from_parts(Config::default(), root_dir, plugin_dir, config_path)
    }

    fn from_parts(
        config: Config,
        root_dir: PathBuf,
        plugin_dir: PathBuf,
        config_path: PathBuf,
    ) -> Result<Self> {
        let base_dirs =
            BaseDirs::new().context("could not resolve the current user's home directory")?;
        fs::create_dir_all(&plugin_dir)
            .with_context(|| format!("failed to create {}", plugin_dir.display()))?;

        let mut plugin_dirs = vec![plugin_dir];
        for configured in &config.plugins.directories {
            plugin_dirs.push(resolve_path(&root_dir, base_dirs.home_dir(), configured));
        }
        dedup_paths(&mut plugin_dirs);

        let mut plugin_search_paths = Vec::new();
        for configured in &config.plugins.search_paths {
            plugin_search_paths.push(resolve_path(&root_dir, base_dirs.home_dir(), configured));
        }
        dedup_paths(&mut plugin_search_paths);

        Ok(Self {
            config,
            config_path,
            plugin_dirs,
            plugin_search_paths,
        })
    }
}

#[cfg(test)]
fn validate_config(config: &Config) -> Result<()> {
    validate_provider_names(&config.ranking.provider_order, "[ranking].provider_order")?;
    validate_provider_names(
        &config.ranking.empty_query_providers,
        "[ranking].empty_query_providers",
    )?;

    for provider in config.ranking.provider_score_boosts.keys() {
        validate_provider_name(provider, "[ranking.provider_score_boosts] key")?;
    }

    for (index, rule) in config.ranking.score_rules.iter().enumerate() {
        let context = format!("[[ranking.score_rules]] entry {}", index + 1);
        validate_provider_names(&rule.providers, &format!("{context}.providers"))?;
    }

    Ok(())
}

fn validate_config_with_spans(config_path: &Path, raw: &str, spans: &RawConfigSpans) -> Result<()> {
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

    Ok(())
}

#[cfg(test)]
fn validate_provider_names(providers: &[String], context: &str) -> Result<()> {
    for provider in providers {
        validate_provider_name(provider, context)?;
    }
    Ok(())
}

#[cfg(test)]
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

fn render_toml_parse_error(config_path: &Path, raw: &str, error: &toml::de::Error) -> String {
    render_config_error_report(
        config_path,
        raw,
        "failed to parse",
        "runx::config::toml_parse",
        error.message(),
        error.span(),
    )
}

fn render_config_validation_error(
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

fn runtime_paths() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let base_dirs =
        BaseDirs::new().context("could not resolve the current user's home directory")?;
    let root_dir = base_dirs.config_dir().join("runx");
    let plugin_dir = root_dir.join("plugins");
    let config_path = root_dir.join("config.toml");
    Ok((root_dir, plugin_dir, config_path))
}

/// Returns the current modification time for the config file, if it exists.
pub fn config_modified_at(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

impl Config {
    /// Converts the configured key/modifier pair into a `global_hotkey` binding.
    pub fn hotkey(&self) -> Result<HotKey> {
        self.hotkey.to_hotkey()
    }

    /// Returns per-plugin config tables with routing metadata stripped out.
    pub fn plugin_config(&self) -> Result<HashMap<String, JsonValue>> {
        self.plugin
            .iter()
            .map(|(id, table)| {
                let mut table = table.clone();
                table.remove("commands");
                let value = serde_json::to_value(table)
                    .with_context(|| format!("failed to serialize plugin config for `{id}`"))?;
                Ok((id.clone(), value))
            })
            .collect()
    }

    /// Returns the command-prefix routing table for configured plugins.
    pub fn plugin_routes(&self) -> Result<HashMap<String, HashMap<String, String>>> {
        let mut routes_by_plugin = HashMap::new();

        for (plugin_id, table) in &self.plugin {
            let Some(commands) = table.get("commands") else {
                continue;
            };

            let commands = commands
                .as_table()
                .ok_or_else(|| anyhow!("`[plugin.{plugin_id}.commands]` must be a TOML table"))?;

            let mut routes = HashMap::new();
            for (command, handler_value) in commands {
                let normalized_command = command.trim();
                if normalized_command.is_empty() {
                    bail!("`[plugin.{plugin_id}.commands]` contains an empty command name");
                }

                let handler = handler_value.as_str().ok_or_else(|| {
                    anyhow!(
                        "`[plugin.{plugin_id}.commands.{normalized_command}]` must be a string handler name"
                    )
                })?;
                let normalized_handler = handler.trim();
                if normalized_handler.is_empty() {
                    bail!("`[plugin.{plugin_id}.commands.{normalized_command}]` must not be empty");
                }

                routes.insert(normalized_command.to_owned(), normalized_handler.to_owned());
            }

            if !routes.is_empty() {
                routes_by_plugin.insert(plugin_id.clone(), routes);
            }
        }

        Ok(routes_by_plugin)
    }
}

impl Default for HotKeyConfig {
    fn default() -> Self {
        Self {
            key: "Space".to_owned(),
            modifiers: vec!["Alt".to_owned()],
        }
    }
}

impl HotKeyConfig {
    /// Parses the config strings into `global_hotkey` types.
    pub fn to_hotkey(&self) -> Result<HotKey> {
        let mut modifiers = Modifiers::empty();
        for modifier in &self.modifiers {
            modifiers |= parse_modifier(modifier)?;
        }
        let code = parse_key(&self.key)?;
        Ok(HotKey::new(Some(modifiers), code))
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 760.0,
            height: 520.0,
            hide_on_blur: true,
            always_on_top: true,
            show_on: WindowDisplayTarget::Primary,
            focus_behavior: WindowFocusBehavior::FocusWindowThenActivateFallback,
        }
    }
}

impl Default for RankingConfig {
    fn default() -> Self {
        Self {
            tie_threshold: 120,
            provider_order: vec![
                "windows".to_owned(),
                "apps".to_owned(),
                "settings".to_owned(),
                "plugins".to_owned(),
                "spotlight".to_owned(),
            ],
            empty_query_providers: vec!["windows".to_owned()],
            provider_score_boosts: HashMap::new(),
            score_rules: Vec::new(),
            result_limit: 24,
        }
    }
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            search_debounce_ms: 24,
            render_coalesce_ms: 8,
        }
    }
}

impl RankingConfig {
    /// Returns the configured tie-break priority for a provider name.
    pub fn provider_rank(&self, provider: &str) -> usize {
        self.provider_order
            .iter()
            .position(|candidate| candidate == provider)
            .unwrap_or(self.provider_order.len() + 1)
    }

    /// Returns the configured additive score adjustment for a provider.
    pub fn provider_score_boost(&self, provider: &str) -> i64 {
        self.provider_score_boosts
            .get(provider)
            .copied()
            .unwrap_or(0)
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            font_family: "\"SF Pro Display\", \"Avenir Next\", \"Helvetica Neue\", sans-serif"
                .to_owned(),
            accent: "#c77b49".to_owned(),
            background: "#f3ede5".to_owned(),
            panel: "#fffaf3".to_owned(),
            text: "#1f1a16".to_owned(),
            muted: "#756759".to_owned(),
            font_sizes: UiFontSizesConfig::default(),
        }
    }
}

impl Default for UiFontSizesConfig {
    fn default() -> Self {
        Self {
            label: 10,
            input: 30,
            title: 16,
            subtitle: 12,
            badge: 11,
            accelerator: 12,
            config_error_title: 24,
            config_error_body: 15,
        }
    }
}

fn resolve_path(root_dir: &Path, home_dir: &Path, raw: &str) -> PathBuf {
    if let Some(stripped) = raw.strip_prefix("~/") {
        return home_dir.join(stripped);
    }

    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        root_dir.join(path)
    }
}

fn dedup_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

fn parse_modifier(value: &str) -> Result<Modifiers> {
    match value.to_ascii_lowercase().as_str() {
        "alt" | "option" => Ok(Modifiers::ALT),
        "control" | "ctrl" => Ok(Modifiers::CONTROL),
        "shift" => Ok(Modifiers::SHIFT),
        "command" | "cmd" | "super" | "meta" => Ok(Modifiers::META),
        other => bail!("unsupported hotkey modifier `{other}` in config.toml"),
    }
}

fn parse_key(value: &str) -> Result<Code> {
    let normalized = value.trim().to_ascii_uppercase();
    if normalized.len() == 1 {
        let Some(ch) = normalized.chars().next() else {
            bail!("hotkey key must not be empty in config.toml");
        };
        return match ch {
            'A' => Ok(Code::KeyA),
            'B' => Ok(Code::KeyB),
            'C' => Ok(Code::KeyC),
            'D' => Ok(Code::KeyD),
            'E' => Ok(Code::KeyE),
            'F' => Ok(Code::KeyF),
            'G' => Ok(Code::KeyG),
            'H' => Ok(Code::KeyH),
            'I' => Ok(Code::KeyI),
            'J' => Ok(Code::KeyJ),
            'K' => Ok(Code::KeyK),
            'L' => Ok(Code::KeyL),
            'M' => Ok(Code::KeyM),
            'N' => Ok(Code::KeyN),
            'O' => Ok(Code::KeyO),
            'P' => Ok(Code::KeyP),
            'Q' => Ok(Code::KeyQ),
            'R' => Ok(Code::KeyR),
            'S' => Ok(Code::KeyS),
            'T' => Ok(Code::KeyT),
            'U' => Ok(Code::KeyU),
            'V' => Ok(Code::KeyV),
            'W' => Ok(Code::KeyW),
            'X' => Ok(Code::KeyX),
            'Y' => Ok(Code::KeyY),
            'Z' => Ok(Code::KeyZ),
            '0' => Ok(Code::Digit0),
            '1' => Ok(Code::Digit1),
            '2' => Ok(Code::Digit2),
            '3' => Ok(Code::Digit3),
            '4' => Ok(Code::Digit4),
            '5' => Ok(Code::Digit5),
            '6' => Ok(Code::Digit6),
            '7' => Ok(Code::Digit7),
            '8' => Ok(Code::Digit8),
            '9' => Ok(Code::Digit9),
            _ => bail!("unsupported hotkey key `{value}` in config.toml"),
        };
    }

    match normalized.as_str() {
        "SPACE" => Ok(Code::Space),
        "ENTER" | "RETURN" => Ok(Code::Enter),
        "ESC" | "ESCAPE" => Ok(Code::Escape),
        "TAB" => Ok(Code::Tab),
        "BACKSPACE" => Ok(Code::Backspace),
        "UP" => Ok(Code::ArrowUp),
        "DOWN" => Ok(Code::ArrowDown),
        "LEFT" => Ok(Code::ArrowLeft),
        "RIGHT" => Ok(Code::ArrowRight),
        other => bail!("unsupported hotkey key `{other}` in config.toml"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        Config, WindowDisplayTarget, WindowFocusBehavior, parse_key, parse_modifier,
        render_toml_parse_error,
    };
    use global_hotkey::hotkey::{Code, Modifiers};

    mod parse_key_tests {
        use super::{Code, parse_key};

        #[test]
        fn accepts_single_letter_keys() {
            assert!(matches!(parse_key("a"), Ok(Code::KeyA)));
        }

        #[test]
        fn rejects_empty_keys() {
            let error = parse_key("").expect_err("empty key should fail");
            assert!(error.to_string().contains("unsupported hotkey key"));
        }
    }

    mod parse_modifier_tests {
        use super::{Modifiers, parse_modifier};

        #[test]
        fn accepts_common_aliases() {
            assert!(matches!(parse_modifier("option"), Ok(Modifiers::ALT)));
            assert!(matches!(parse_modifier("cmd"), Ok(Modifiers::META)));
        }
    }

    mod window_display_target_tests {
        use super::{Config, WindowDisplayTarget};

        #[test]
        fn defaults_to_primary() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(config.window.show_on, WindowDisplayTarget::Primary);
        }

        #[test]
        fn accepts_cursor() {
            let cursor: Config =
                toml::from_str("[window]\nshow_on = \"cursor\"\n").expect("cursor should parse");
            assert_eq!(cursor.window.show_on, WindowDisplayTarget::Cursor);
        }
    }

    mod window_focus_behavior_tests {
        use super::{Config, WindowFocusBehavior};

        #[test]
        fn defaults_to_focus_then_activate_fallback() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(
                config.window.focus_behavior,
                WindowFocusBehavior::FocusWindowThenActivateFallback
            );
        }

        #[test]
        fn accepts_activate_app_only() {
            let config: Config =
                toml::from_str("[window]\nfocus_behavior = \"activate_app_only\"\n")
                    .expect("focus behavior should parse");
            assert_eq!(
                config.window.focus_behavior,
                WindowFocusBehavior::ActivateAppOnly
            );
        }

        #[test]
        fn accepts_focus_window_only() {
            let config: Config =
                toml::from_str("[window]\nfocus_behavior = \"focus_window_only\"\n")
                    .expect("focus behavior should parse");
            assert_eq!(
                config.window.focus_behavior,
                WindowFocusBehavior::FocusWindowOnly
            );
        }
    }

    mod empty_query_provider_tests {
        use super::Config;
        use crate::config::{
            RankingScoreRule, RankingScoreRuleField, RankingScoreRuleMatchKind, validate_config,
        };

        #[test]
        fn defaults_to_windows_only() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(config.ranking.empty_query_providers, vec!["windows"]);
        }

        #[test]
        fn accepts_custom_provider_list() {
            let config: Config =
                toml::from_str("[ranking]\nempty_query_providers = [\"windows\", \"plugins\"]\n")
                    .expect("custom empty query providers should parse");
            assert_eq!(
                config.ranking.empty_query_providers,
                vec!["windows", "plugins"]
            );
        }

        #[test]
        fn accepts_provider_score_boosts() {
            let config: Config =
                toml::from_str("[ranking.provider_score_boosts]\nspotlight = -150\napps = 60\n")
                    .expect("provider score boosts should parse");
            assert_eq!(config.ranking.provider_score_boost("spotlight"), -150);
            assert_eq!(config.ranking.provider_score_boost("apps"), 60);
            assert_eq!(config.ranking.provider_score_boost("windows"), 0);
        }

        #[test]
        fn accepts_score_rules() {
            let config: Config = toml::from_str(
                "[[ranking.score_rules]]\nproviders = [\"apps\", \"windows\"]\nfield = \"title\"\nmatch = \"contains\"\npattern = \"spotify\"\nboost = 120\n",
            )
            .expect("score rules should parse");
            let rule = &config.ranking.score_rules[0];
            assert_eq!(rule.providers, vec!["apps", "windows"]);
            assert_eq!(rule.field, super::super::RankingScoreRuleField::Title);
            assert_eq!(
                rule.match_kind,
                super::super::RankingScoreRuleMatchKind::Contains
            );
            assert_eq!(rule.pattern, "spotify");
            assert_eq!(rule.boost, 120);
        }

        #[test]
        fn rejects_unknown_top_level_section() {
            let error =
                toml::from_str::<Config>("[windo]\nwidth = 760\n").expect_err("typo should fail");
            assert!(error.message().contains("unknown field `windo`"));
        }

        #[test]
        fn rejects_unknown_nested_field() {
            let error = toml::from_str::<Config>("[window]\nwidt = 760\n")
                .expect_err("unknown nested field should fail");
            assert!(error.message().contains("unknown field `widt`"));
        }

        #[test]
        fn rejects_unknown_provider_in_provider_order() {
            let mut config = Config::default();
            config.ranking.provider_order = vec!["windows".to_owned(), "asdfasdf".to_owned()];

            let error = validate_config(&config).expect_err("unknown provider should fail");
            assert!(
                error
                    .to_string()
                    .contains("[ranking].provider_order contains unknown provider `asdfasdf`")
            );
        }

        #[test]
        fn rejects_unknown_provider_in_empty_query_providers() {
            let mut config = Config::default();
            config.ranking.empty_query_providers = vec!["asdfasdf".to_owned()];

            let error = validate_config(&config).expect_err("unknown provider should fail");
            assert!(
                error.to_string().contains(
                    "[ranking].empty_query_providers contains unknown provider `asdfasdf`"
                )
            );
        }

        #[test]
        fn rejects_unknown_provider_in_provider_score_boosts() {
            let mut config = Config::default();
            config
                .ranking
                .provider_score_boosts
                .insert("asdfasdf".to_owned(), 10);

            let error = validate_config(&config).expect_err("unknown provider should fail");
            assert!(error.to_string().contains(
                "[ranking.provider_score_boosts] key contains unknown provider `asdfasdf`"
            ));
        }

        #[test]
        fn rejects_unknown_provider_in_score_rules() {
            let mut config = Config::default();
            config.ranking.score_rules = vec![RankingScoreRule {
                providers: vec!["asdfasdf".to_owned()],
                field: RankingScoreRuleField::Title,
                match_kind: RankingScoreRuleMatchKind::Contains,
                pattern: "spotify".to_owned(),
                boost: 120,
            }];

            let error = validate_config(&config).expect_err("unknown provider should fail");
            assert!(error.to_string().contains(
                "[[ranking.score_rules]] entry 1.providers contains unknown provider `asdfasdf`"
            ));
        }
    }

    mod timing_tests {
        use super::Config;

        #[test]
        fn defaults_to_current_search_and_render_timings() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(config.timing.search_debounce_ms, 24);
            assert_eq!(config.timing.render_coalesce_ms, 8);
        }

        #[test]
        fn accepts_custom_timing_values() {
            let config: Config =
                toml::from_str("[timing]\nsearch_debounce_ms = 32\nrender_coalesce_ms = 12\n")
                    .expect("custom timing values should parse");
            assert_eq!(config.timing.search_debounce_ms, 32);
            assert_eq!(config.timing.render_coalesce_ms, 12);
        }
    }

    mod ui_tests {
        use super::Config;

        #[test]
        fn defaults_to_current_ui_font_sizes() {
            let config: Config = toml::from_str("").expect("empty config should parse");
            assert_eq!(config.ui.font_sizes.label, 10);
            assert_eq!(config.ui.font_sizes.input, 30);
            assert_eq!(config.ui.font_sizes.title, 16);
            assert_eq!(config.ui.font_sizes.subtitle, 12);
            assert_eq!(config.ui.font_sizes.badge, 11);
            assert_eq!(config.ui.font_sizes.accelerator, 12);
            assert_eq!(config.ui.font_sizes.config_error_title, 24);
            assert_eq!(config.ui.font_sizes.config_error_body, 15);
        }

        #[test]
        fn accepts_custom_ui_font_sizes() {
            let config: Config = toml::from_str(
                "[ui.font_sizes]\ninput = 34\ntitle = 18\nsubtitle = 13\nbadge = 12\naccelerator = 13\nconfig_error_title = 28\nconfig_error_body = 17\nlabel = 11\n",
            )
            .expect("custom ui font sizes should parse");

            assert_eq!(config.ui.font_sizes.label, 11);
            assert_eq!(config.ui.font_sizes.input, 34);
            assert_eq!(config.ui.font_sizes.title, 18);
            assert_eq!(config.ui.font_sizes.subtitle, 13);
            assert_eq!(config.ui.font_sizes.badge, 12);
            assert_eq!(config.ui.font_sizes.accelerator, 13);
            assert_eq!(config.ui.font_sizes.config_error_title, 28);
            assert_eq!(config.ui.font_sizes.config_error_body, 17);
        }
    }

    mod diagnostics_tests {
        use super::{Config, Path, render_toml_parse_error};
        use crate::config::{RawConfigSpans, validate_config_with_spans};

        #[test]
        fn toml_parse_errors_include_source_context() {
            let raw = "[[ranking.score_rules]]\nboost = \"oops\"\n";
            let error = toml::from_str::<Config>(raw).expect_err("config should fail to parse");
            let rendered = render_toml_parse_error(Path::new("/tmp/runx-config.toml"), raw, &error);

            assert!(rendered.contains("failed to parse /tmp/runx-config.toml"));
            assert!(rendered.contains("boost = \"oops\""));
            assert!(rendered.contains("expected i64"));
        }

        #[test]
        fn validation_errors_include_source_context() {
            let raw = "[ranking]\nprovider_order = [\"windows\", \"asdfasdf\"]\n";
            let spans: RawConfigSpans =
                toml::from_str(raw).expect("raw spans config should parse structurally");
            let rendered =
                validate_config_with_spans(Path::new("/tmp/runx-config.toml"), raw, &spans)
                    .expect_err("validation should fail")
                    .to_string();

            assert!(rendered.contains("invalid configuration /tmp/runx-config.toml"));
            assert!(rendered.contains("provider_order = [\"windows\", \"asdfasdf\"]"));
            assert!(rendered.contains("unknown provider `asdfasdf`"));
        }
    }
}
