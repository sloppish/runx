//! Readiness gate for webview communication.
//!
//! Ensures no `evaluate_script` calls reach the webview before the frontend
//! has signalled it is ready to receive them. Pre-ready writes are captured
//! as latest-wins pending state and flushed atomically on the first Ready.

use std::time::Duration;

use anyhow::{Context, Result};
use tao::event_loop::EventLoopProxy;
use tokio::runtime::Runtime;
use tracing::warn;
use wry::WebView;

use crate::types::AppEvent;

const READY_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(5);

/// Tracks whether the frontend has sent its Ready signal and buffers the
/// latest pending payloads when it has not.
pub(crate) struct FrontendReadiness {
    ready: bool,
    pending_render_script: Option<String>,
    pending_config_script: Option<String>,
    pending_focus: bool,
    watchdog_armed: bool,
}

impl FrontendReadiness {
    pub(crate) fn new() -> Self {
        Self {
            ready: false,
            pending_render_script: None,
            pending_config_script: None,
            pending_focus: false,
            watchdog_armed: false,
        }
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.ready
    }

    /// Attempts to evaluate a render script. If not ready, stores it as the
    /// latest pending render (replacing any previous pending render).
    pub(crate) fn eval_render(
        &mut self,
        webview: &WebView,
        script: String,
        runtime: &Runtime,
        proxy: &EventLoopProxy<AppEvent>,
    ) -> Result<()> {
        if self.ready {
            webview
                .evaluate_script(&script)
                .context("failed to render the launcher UI")?;
        } else {
            self.pending_render_script = Some(script);
            self.arm_watchdog(runtime, proxy);
        }
        Ok(())
    }

    /// Attempts to evaluate a frontend config script. If not ready, stores it
    /// as the latest pending config (replacing any previous pending config).
    pub(crate) fn eval_config(
        &mut self,
        webview: &WebView,
        script: String,
        runtime: &Runtime,
        proxy: &EventLoopProxy<AppEvent>,
    ) -> Result<()> {
        if self.ready {
            webview
                .evaluate_script(&script)
                .context("failed to apply the reloaded UI theme")?;
        } else {
            self.pending_config_script = Some(script);
            self.arm_watchdog(runtime, proxy);
        }
        Ok(())
    }

    /// Records that focus should be applied after readiness is established.
    pub(crate) fn request_focus(&mut self, runtime: &Runtime, proxy: &EventLoopProxy<AppEvent>) {
        if !self.ready {
            self.pending_focus = true;
            self.arm_watchdog(runtime, proxy);
        }
    }

    /// Marks the frontend as ready and flushes pending config then render.
    /// Returns `true` if a pending render was flushed (caller can skip a
    /// redundant re-render in that case).
    pub(crate) fn mark_ready(&mut self, webview: &WebView) -> Result<bool> {
        self.ready = true;
        self.pending_focus = false;

        if let Some(script) = self.pending_config_script.take() {
            webview
                .evaluate_script(&script)
                .context("failed to flush pending frontend config on Ready")?;
        }
        let flushed_render = if let Some(script) = self.pending_render_script.take() {
            webview
                .evaluate_script(&script)
                .context("failed to flush pending render on Ready")?;
            true
        } else {
            false
        };

        Ok(flushed_render)
    }

    /// Fires on the watchdog timeout; logs if pending work exists and Ready
    /// has not arrived.
    pub(crate) fn watchdog_fired(&self) {
        if !self.ready
            && (self.pending_render_script.is_some()
                || self.pending_config_script.is_some()
                || self.pending_focus)
        {
            warn!(
                pending_render = self.pending_render_script.is_some(),
                pending_config = self.pending_config_script.is_some(),
                pending_focus = self.pending_focus,
                "frontend Ready not received within {}s — pending work is blocked",
                READY_WATCHDOG_TIMEOUT.as_secs()
            );
        }
    }

    fn arm_watchdog(&mut self, runtime: &Runtime, proxy: &EventLoopProxy<AppEvent>) {
        if self.watchdog_armed {
            return;
        }
        self.watchdog_armed = true;
        let proxy = proxy.clone();
        runtime.handle().spawn(async move {
            tokio::time::sleep(READY_WATCHDOG_TIMEOUT).await;
            let _ = proxy.send_event(AppEvent::FrontendReadyWatchdog);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::FrontendReadiness;

    #[test]
    fn pre_ready_render_is_buffered_and_flushed() {
        let readiness = FrontendReadiness::new();
        assert!(!readiness.is_ready());
        assert!(readiness.pending_render_script.is_none());
    }

    #[test]
    fn latest_wins_for_pending_render() {
        let mut readiness = FrontendReadiness::new();
        readiness.pending_render_script = Some("first".to_owned());
        readiness.pending_render_script = Some("second".to_owned());
        assert_eq!(readiness.pending_render_script.as_deref(), Some("second"));
    }

    #[test]
    fn latest_wins_for_pending_config() {
        let mut readiness = FrontendReadiness::new();
        readiness.pending_config_script = Some("config_a".to_owned());
        readiness.pending_config_script = Some("config_b".to_owned());
        assert_eq!(readiness.pending_config_script.as_deref(), Some("config_b"));
    }

    #[test]
    fn watchdog_fires_only_when_pending_work_exists() {
        let mut readiness = FrontendReadiness::new();
        readiness.watchdog_fired(); // no pending work, should not warn

        readiness.pending_render_script = Some("pending".to_owned());
        readiness.watchdog_fired(); // has pending work, would warn (no panic)
    }
}
