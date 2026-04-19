use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

#[derive(Debug, Clone)]
pub struct SearchItem {
    pub id: String,
    pub provider: String,
    pub badge: String,
    pub icon: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub raw_score: i64,
    pub action: Action,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginActionPayload {
    pub kind: String,
    #[serde(flatten)]
    pub fields: JsonMap<String, JsonValue>,
}

impl PluginActionPayload {
    pub fn as_json(&self) -> JsonValue {
        let mut object = self.fields.clone();
        object.insert("kind".to_owned(), JsonValue::String(self.kind.clone()));
        JsonValue::Object(object)
    }

    pub fn likely_needs_accessibility(&self) -> bool {
        let kind = self.kind.as_str();
        kind == "type"
            || kind.starts_with("type_")
            || kind.ends_with("_type")
            || kind.contains("keystroke")
    }
}

#[derive(Debug, Clone)]
pub enum Action {
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
    pub fn likely_needs_accessibility(&self) -> bool {
        match self {
            Self::FocusWindow { .. } => true,
            Self::Plugin { payload, .. } => payload.likely_needs_accessibility(),
            Self::OpenApplication { .. } | Self::OpenPath { .. } | Self::OpenSettings { .. } => {
                false
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Frontend(FrontendCommand),
    Render,
    StartSearch {
        token: u64,
    },
    TrayToggle,
    TrayOpen,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontendCommand {
    Ready,
    QueryChanged { query: String },
    Activate { index: usize },
    Hide,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewState {
    pub query: String,
    pub items: Vec<ViewItem>,
    pub status: Option<StatusLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewItem {
    pub title: String,
    pub subtitle: String,
    pub badge: String,
    pub icon: Option<String>,
    pub accelerator: Option<String>,
    pub compact: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusLine {
    pub kind: &'static str,
    pub message: String,
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
