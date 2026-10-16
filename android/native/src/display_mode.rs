//! Debounce physical display mode changes before rebinding capture.
use std::time::{Duration, Instant};

pub(crate) struct DisplayModeWatch {
    applied: i32,
    pending: Option<(i32, Instant)>,
}

impl DisplayModeWatch {
    pub fn new(mode: i32) -> Self {
        Self {
            applied: mode,
            pending: None,
        }
    }

    pub fn changed(&mut self, mode: i32, now: Instant) -> bool {
        if mode == self.applied {
            self.pending = None;
            return false;
        }
        match self.pending {
            Some((pending, since)) if pending == mode => {
                if now.saturating_duration_since(since) >= Duration::from_millis(500) {
                    self.applied = mode;
                    self.pending = None;
                    return true;
                }
            }
            _ => self.pending = Some((mode, now)),
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_mode_change_is_reported_once_after_settling() {
        let now = Instant::now();
        let mut watch = DisplayModeWatch::new(1);
        assert!(!watch.changed(2, now));
        assert!(!watch.changed(2, now + Duration::from_millis(499)));
        assert!(watch.changed(2, now + Duration::from_millis(500)));
        assert!(!watch.changed(2, now + Duration::from_secs(1)));
    }

    #[test]
    fn returning_to_applied_mode_cancels_pending_change() {
        let now = Instant::now();
        let mut watch = DisplayModeWatch::new(1);
        assert!(!watch.changed(2, now));
        assert!(!watch.changed(1, now + Duration::from_millis(400)));
        assert!(!watch.changed(2, now + Duration::from_secs(1)));
        assert!(watch.changed(2, now + Duration::from_millis(1500)));
    }

    #[test]
    fn intermediate_mode_restarts_settling_timer() {
        let now = Instant::now();
        let mut watch = DisplayModeWatch::new(1);
        assert!(!watch.changed(2, now));
        assert!(!watch.changed(3, now + Duration::from_millis(400)));
        assert!(!watch.changed(3, now + Duration::from_millis(500)));
        assert!(watch.changed(3, now + Duration::from_millis(900)));
    }
}
