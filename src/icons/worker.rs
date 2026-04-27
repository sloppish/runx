use std::sync::{Arc, Condvar, Mutex};

use super::{MAX_ICON_RENDER_WORKERS, lock_or_recover};

pub(super) struct RenderLimiter {
    active: Mutex<usize>,
    available: Condvar,
}

pub(super) struct RenderPermit {
    limiter: Arc<RenderLimiter>,
}

impl RenderLimiter {
    pub(super) fn new() -> Self {
        Self {
            active: Mutex::new(0),
            available: Condvar::new(),
        }
    }

    pub(super) fn acquire(self: &Arc<Self>) -> RenderPermit {
        let mut active = lock_or_recover(&self.active);
        while *active >= MAX_ICON_RENDER_WORKERS {
            active = self
                .available
                .wait(active)
                .unwrap_or_else(|poison| poison.into_inner());
        }
        *active += 1;
        RenderPermit {
            limiter: Arc::clone(self),
        }
    }
}

impl Drop for RenderPermit {
    fn drop(&mut self) {
        let mut active = lock_or_recover(&self.limiter.active);
        *active = active.saturating_sub(1);
        self.limiter.available.notify_one();
    }
}
