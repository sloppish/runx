use anyhow::{Result, bail};
use serde::Deserialize;

use crate::types::PluginActionPayload;

use super::LuaPlugin;

#[derive(Debug, Deserialize)]
pub(super) struct PluginItemWire {
    id: Option<String>,
    title: String,
    subtitle: Option<String>,
    score: Option<i64>,
    badge: Option<String>,
    icon: Option<String>,
    style: Option<String>,
    payload: PluginActionPayload,
}

#[derive(Debug)]
pub(super) struct PluginItem {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) score: i64,
    pub(super) badge: String,
    pub(super) icon: Option<String>,
    pub(super) compact: bool,
    pub(super) payload: PluginActionPayload,
}

pub(super) fn validate_plugin_item(
    plugin: &LuaPlugin,
    index: usize,
    item: PluginItemWire,
) -> Result<PluginItem> {
    let title = item.title.trim();
    if title.is_empty() {
        bail!("plugin item {} must have a non-empty title", index + 1);
    }

    let id = match item.id {
        Some(id) => {
            let id = id.trim();
            if id.is_empty() {
                bail!("plugin item {} has an empty `id`", index + 1);
            }
            id.to_owned()
        }
        None => format!("plugin:{}:{}", plugin.id, title),
    };

    let compact = match item.style {
        Some(style) => match style.trim() {
            "compact" => true,
            "full" => false,
            "" => bail!("plugin item {} has an empty `style`", index + 1),
            other => bail!(
                "plugin item {} has invalid `style` `{other}`; expected `compact` or `full`",
                index + 1
            ),
        },
        None => true,
    };

    let badge = match item.badge {
        Some(badge) => {
            let badge = badge.trim();
            if badge.is_empty() {
                bail!("plugin item {} has an empty `badge`", index + 1);
            }
            badge.to_owned()
        }
        None if compact => String::new(),
        None => plugin.badge.clone(),
    };

    let subtitle = item
        .subtitle
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(if compact { "" } else { plugin.name.as_str() })
        .to_owned();

    let icon = item
        .icon
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    Ok(PluginItem {
        id,
        title: title.to_owned(),
        subtitle,
        score: item.score.unwrap_or(0),
        badge,
        icon,
        compact,
        payload: item.payload,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{PluginItemWire, validate_plugin_item};
    use crate::plugins::LuaPlugin;

    fn test_plugin() -> LuaPlugin {
        LuaPlugin {
            id: "test".to_owned(),
            name: "Test Plugin".to_owned(),
            badge: "TST".to_owned(),
            path: PathBuf::from("test/init.lua"),
            source: String::new(),
        }
    }

    #[test]
    fn validate_plugin_item_rejects_empty_title() {
        let item = serde_json::from_value::<PluginItemWire>(json!({
            "title": "   ",
            "payload": { "kind": "copy" }
        }))
        .expect("valid wire item");

        let error = validate_plugin_item(&test_plugin(), 0, item)
            .expect_err("empty titles should fail")
            .to_string();

        assert!(error.contains("non-empty title"));
    }

    #[test]
    fn validate_plugin_item_falls_back_to_plugin_defaults() {
        let item = serde_json::from_value::<PluginItemWire>(json!({
            "title": "Copy secret",
            "payload": { "kind": "copy_secret" }
        }))
        .expect("valid wire item");

        let item = validate_plugin_item(&test_plugin(), 0, item).expect("validated item");

        assert_eq!(item.id, "plugin:test:Copy secret");
        assert_eq!(item.badge, "");
        assert_eq!(item.subtitle, "");
        assert!(item.compact);
    }

    #[test]
    fn validate_plugin_item_accepts_full_style() {
        let item = serde_json::from_value::<PluginItemWire>(json!({
            "title": "Copy secret",
            "style": "full",
            "payload": { "kind": "copy_secret" }
        }))
        .expect("valid wire item");

        let item = validate_plugin_item(&test_plugin(), 0, item).expect("validated item");
        assert!(!item.compact);
        assert_eq!(item.badge, "TST");
        assert_eq!(item.subtitle, "Test Plugin");
    }

    #[test]
    fn validate_plugin_item_rejects_unknown_style() {
        let item = serde_json::from_value::<PluginItemWire>(json!({
            "title": "Copy secret",
            "style": "wide",
            "payload": { "kind": "copy_secret" }
        }))
        .expect("valid wire item");

        let error = validate_plugin_item(&test_plugin(), 0, item)
            .expect_err("invalid style should fail")
            .to_string();

        assert!(error.contains("expected `compact` or `full`"));
    }
}
