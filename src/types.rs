//! Shared data structures exchanged between modules.
//!
//! These types are the common language between the launcher runtime, providers,
//! plugins, and the embedded frontend.

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

/// Canonical internal representation of one search result candidate.
#[derive(Debug, Clone)]
pub struct SearchItem {
    pub id: String,
    pub provider: String,
    pub badge: String,
    pub icon: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub compact: bool,
    pub raw_score: i64,
    pub action: Action,
}

/// Validated plugin action payload passed from Lua back into Rust.
///
/// The payload contract stays intentionally small: a required `kind` plus
/// arbitrary extra JSON-like fields preserved for the plugin's `run(action)`
/// handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginActionPayload {
    pub kind: String,
    #[serde(flatten)]
    pub fields: JsonMap<String, JsonValue>,
}

impl PluginActionPayload {
    /// Converts the payload back into a JSON object for passing into Lua.
    pub fn as_json(&self) -> JsonValue {
        let mut object = self.fields.clone();
        object.insert("kind".to_owned(), JsonValue::String(self.kind.clone()));
        JsonValue::Object(object)
    }

    /// Validates the minimal contract Runx expects from plugin action payloads.
    pub fn validate(&self) -> Result<(), String> {
        if self.kind.trim().is_empty() {
            return Err("action.kind must not be empty".to_owned());
        }

        if self.kind.trim() != self.kind {
            return Err("action.kind must not contain leading or trailing whitespace".to_owned());
        }

        if self.fields.contains_key("kind") {
            return Err("action fields must not contain the reserved key `kind`".to_owned());
        }

        if self.fields.keys().any(|key| key.trim().is_empty()) {
            return Err("action field names must not be empty".to_owned());
        }

        Ok(())
    }

    /// Heuristic used to preflight Accessibility for typing-like actions.
    pub fn likely_needs_accessibility(&self) -> bool {
        let kind = self.kind.as_str();
        kind == "type"
            || kind.starts_with("type_")
            || kind.ends_with("_type")
            || kind.contains("keystroke")
    }
}

/// Actions that Runx can execute after a search result is activated.
#[derive(Debug, Clone)]
pub enum Action {
    Noop,
    OpenApplication {
        path: String,
    },
    OpenPath {
        path: String,
    },
    OpenSettings {
        url: String,
        title: String,
    },
    FocusWindow {
        app_name: String,
        window_title: String,
    },
    Plugin {
        plugin_id: String,
        payload: PluginActionPayload,
    },
}

impl Action {
    /// Returns whether executing this action is likely to require Accessibility.
    pub fn likely_needs_accessibility(&self) -> bool {
        match self {
            Self::Noop => false,
            Self::FocusWindow { .. } => false,
            Self::Plugin { payload, .. } => payload.likely_needs_accessibility(),
            Self::OpenApplication { .. } | Self::OpenPath { .. } | Self::OpenSettings { .. } => {
                false
            }
        }
    }
}

/// User events sent through Tao's custom event channel.
#[derive(Debug, Clone)]
pub enum AppEvent {
    Frontend(FrontendCommand),
    Render,
    StartSearch {
        token: u64,
    },
    TrayToggle,
    TrayOpen,
    TrayToggleAutostart,
    Quit,
    ProviderItems {
        generation: u64,
        provider: String,
        items: Vec<SearchItem>,
    },
    ProviderError {
        generation: u64,
        provider: String,
        message: String,
    },
    ActionOutcome {
        message: String,
        is_error: bool,
    },
}

/// Commands emitted by the embedded frontend back into the Rust event loop.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontendCommand {
    Ready,
    QueryChanged { query: String },
    Activate { index: usize },
    Hide,
}

/// Serialized frontend state pushed into the webview on each render.
#[derive(Debug, Clone, Serialize)]
pub struct ViewState {
    pub query: String,
    pub config_error: Option<String>,
    pub items: Vec<ViewItem>,
}

/// One visible row in the launcher result list.
#[derive(Debug, Clone, Serialize)]
pub struct ViewItem {
    pub title: String,
    pub subtitle: String,
    pub badge: String,
    pub icon: Option<String>,
    pub accelerator: Option<String>,
    pub compact: bool,
}

#[cfg(test)]
mod tests {
    use super::{Action, PluginActionPayload};

    #[test]
    fn plugin_action_payload_requires_kind() {
        let error = serde_json::from_value::<PluginActionPayload>(serde_json::json!({
            "entry": "mail/example"
        }))
        .expect_err("payload without kind should fail");

        assert!(error.to_string().contains("kind"));
    }

    #[test]
    fn plugin_action_payload_preserves_extra_fields() {
        let payload = serde_json::from_value::<PluginActionPayload>(serde_json::json!({
            "kind": "copy_password",
            "entry": "mail/example",
            "count": 2
        }))
        .expect("valid payload");

        assert_eq!(payload.kind, "copy_password");
        assert_eq!(
            payload.fields.get("entry"),
            Some(&serde_json::json!("mail/example"))
        );
        assert_eq!(payload.fields.get("count"), Some(&serde_json::json!(2)));
    }

    #[test]
    fn plugin_action_payload_detects_typing_actions() {
        let payload = serde_json::from_value::<PluginActionPayload>(serde_json::json!({
            "kind": "type_otp",
            "entry": "mail/example"
        }))
        .expect("valid payload");

        assert!(payload.likely_needs_accessibility());
    }

    #[test]
    fn plugin_action_payload_rejects_empty_kind() {
        let payload = serde_json::from_value::<PluginActionPayload>(serde_json::json!({
            "kind": "   "
        }))
        .expect("deserializes before validation");

        let error = payload.validate().expect_err("empty kind should fail");
        assert!(error.contains("must not be empty"));
    }

    #[test]
    fn plugin_action_payload_rejects_whitespace_padded_kind() {
        let payload = serde_json::from_value::<PluginActionPayload>(serde_json::json!({
            "kind": " copy_password "
        }))
        .expect("deserializes before validation");

        let error = payload
            .validate()
            .expect_err("whitespace-padded kind should fail");
        assert!(error.contains("leading or trailing whitespace"));
    }

    #[test]
    fn plugin_action_payload_validation_accepts_normal_kind() {
        let payload = serde_json::from_value::<PluginActionPayload>(serde_json::json!({
            "kind": "copy_password",
            "entry": "mail/example"
        }))
        .expect("valid payload");

        payload.validate().expect("validation should pass");
    }

    #[test]
    fn action_detects_plugin_typing_preflight() {
        let payload = serde_json::from_value::<PluginActionPayload>(serde_json::json!({
            "kind": "generate_type",
            "args": "mail/example 20"
        }))
        .expect("valid payload");

        let action = Action::Plugin {
            plugin_id: "pass".to_owned(),
            payload,
        };

        assert!(action.likely_needs_accessibility());
    }
}
