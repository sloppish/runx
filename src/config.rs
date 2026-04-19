use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use directories::BaseDirs;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use toml::{Table, Value as TomlValue};

const DEFAULT_CONFIG: &str = r##"# Runx configuration
#
# `provider_order` controls which provider wins when scores are close.
# `tie_threshold` is the raw fuzzy-score delta that still counts as "similar".

[hotkey]
key = "Space"
modifiers = ["Alt"]

[window]
width = 760
height = 520
hide_on_blur = true
always_on_top = true

[ranking]
tie_threshold = 120
provider_order = ["windows", "apps", "settings", "plugins", "spotlight"]
result_limit = 24

[plugins]
directories = []

# Per-plugin configuration can live under `[plugin.<id>]`.
# Example:
# [plugin.pass]
# pass_rank_bin = "/absolute/path/to/pass_rank"
#
[ui]
font_family = "\"SF Pro Display\", \"Avenir Next\", \"Helvetica Neue\", sans-serif"
accent = "#c77b49"
background = "#f3ede5"
panel = "#fffaf3"
text = "#1f1a16"
muted = "#756759"
"##;
const DEFAULT_PASS_PLUGIN_RESOURCE: &str = "defaults/pass.lua";
const DEFAULT_PASS_RANK_RESOURCE: &str = "defaults/pass/pass_rank";

pub struct LoadedConfig {
    pub config: Config,
    pub plugin_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkey: HotKeyConfig,
    pub window: WindowConfig,
    pub ranking: RankingConfig,
    pub plugins: PluginsConfig,
    pub plugin: HashMap<String, Table>,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HotKeyConfig {
    pub key: String,
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub width: f64,
    pub height: f64,
    pub hide_on_blur: bool,
    pub always_on_top: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RankingConfig {
    pub tie_threshold: i64,
    pub provider_order: Vec<String>,
    pub result_limit: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PluginsConfig {
    pub directories: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub font_family: String,
    pub accent: String,
    pub background: String,
    pub panel: String,
    pub text: String,
    pub muted: String,
}

impl LoadedConfig {
    pub fn load() -> Result<Self> {
        let base_dirs =
            BaseDirs::new().context("could not resolve the current user's home directory")?;
        let root_dir = base_dirs.config_dir().join("runx");
        let plugin_dir = root_dir.join("plugins");
        fs::create_dir_all(&plugin_dir)
            .with_context(|| format!("failed to create {}", plugin_dir.display()))?;
        let pass_asset_dir = plugin_dir.join("pass");
        fs::create_dir_all(&pass_asset_dir)
            .with_context(|| format!("failed to create {}", pass_asset_dir.display()))?;

        let config_path = root_dir.join("config.toml");
        if !config_path.exists() {
            fs::write(&config_path, DEFAULT_CONFIG)
                .with_context(|| format!("failed to write {}", config_path.display()))?;
        }

        let default_plugin_path = plugin_dir.join("pass.lua");
        if !default_plugin_path.exists() {
            write_default_plugin(&default_plugin_path)
                .with_context(|| format!("failed to write {}", default_plugin_path.display()))?;
        }

        let pass_rank_path = pass_asset_dir.join("pass_rank");
        if !pass_rank_path.exists() {
            install_resource_if_available(DEFAULT_PASS_RANK_RESOURCE, &pass_rank_path)
                .with_context(|| format!("failed to install {}", pass_rank_path.display()))?;
        }

        let raw = fs::read_to_string(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?;
        let mut config: Config = toml::from_str(&raw).with_context(|| {
            format!(
                "failed to parse {}.\nCheck the TOML syntax and the documented field names.",
                config_path.display()
            )
        })?;
        inject_default_pass_config(&mut config, &pass_rank_path);

        let mut plugin_dirs = vec![plugin_dir];
        for configured in &config.plugins.directories {
            plugin_dirs.push(resolve_path(&root_dir, base_dirs.home_dir(), configured));
        }
        dedup_paths(&mut plugin_dirs);

        Ok(Self {
            config,
            plugin_dirs,
        })
    }
}

fn write_default_plugin(destination: &Path) -> Result<()> {
    if install_resource_if_available(DEFAULT_PASS_PLUGIN_RESOURCE, destination)? {
        return Ok(());
    }

    fs::write(destination, include_str!("../plugins/pass.lua")).with_context(|| {
        format!(
            "failed to write bundled pass plugin to {}",
            destination.display()
        )
    })
}

fn install_resource_if_available(resource: &str, destination: &Path) -> Result<bool> {
    let Some(source) = app_resource_path(resource) else {
        return Ok(false);
    };

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    fs::copy(&source, destination).with_context(|| {
        format!(
            "failed to copy bundled resource {} to {}",
            source.display(),
            destination.display()
        )
    })?;

    #[cfg(unix)]
    if source
        .extension()
        .and_then(|value| value.to_str())
        .is_none()
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(destination, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("failed to mark {} executable", destination.display()))?;
    }

    Ok(true)
}

fn app_resource_path(relative: &str) -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let contents_dir = executable.parent()?.parent()?;
    let resource_path = contents_dir.join("Resources").join(relative);
    resource_path.exists().then_some(resource_path)
}

fn inject_default_pass_config(config: &mut Config, pass_rank_path: &Path) {
    if !pass_rank_path.exists() {
        return;
    }

    let plugin_table = config.plugin.entry("pass".to_owned()).or_default();
    if plugin_table.contains_key("pass_rank_bin") {
        return;
    }

    plugin_table.insert(
        "pass_rank_bin".to_owned(),
        TomlValue::String(pass_rank_path.to_string_lossy().to_string()),
    );
}

impl Config {
    pub fn hotkey(&self) -> Result<HotKey> {
        self.hotkey.to_hotkey()
    }

    pub fn plugin_config(&self) -> Result<HashMap<String, JsonValue>> {
        self.plugin
            .iter()
            .map(|(id, table)| {
                let value = serde_json::to_value(table)
                    .with_context(|| format!("failed to serialize plugin config for `{id}`"))?;
                Ok((id.clone(), value))
            })
            .collect()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: HotKeyConfig::default(),
            window: WindowConfig::default(),
            ranking: RankingConfig::default(),
            plugins: PluginsConfig::default(),
            plugin: HashMap::default(),
            ui: UiConfig::default(),
        }
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
            result_limit: 24,
        }
    }
}

impl RankingConfig {
    pub fn provider_rank(&self, provider: &str) -> usize {
        self.provider_order
            .iter()
            .position(|candidate| candidate == provider)
            .unwrap_or(self.provider_order.len() + 1)
    }
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            directories: Vec::new(),
        }
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
        let ch = normalized.chars().next().unwrap();
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
