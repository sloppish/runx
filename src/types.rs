use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

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
        payload: JsonValue,
    },
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Frontend(FrontendCommand),
    Render,
    StartSearch {
        token: u64,
    },
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
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusLine {
    pub kind: &'static str,
    pub message: String,
}
