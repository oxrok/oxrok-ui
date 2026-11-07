//! Injectable monotonic clock: the app reads time only through
//! [`Clock`], so tests can drive animations and timers deterministically.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub trait Clock: Send + Sync {
    /// Time elapsed since an arbitrary, fixed epoch.
    fn now(&self) -> Duration;
}

/// Wall-time clock backed by [`Instant`].
#[derive(Debug, Clone)]
pub struct MonotonicClock {
    epoch: Instant,
}

impl MonotonicClock {
    pub fn new() -> Self {
        Self { epoch: Instant::now() }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now(&self) -> Duration {
        self.epoch.elapsed()
    }
}

/// Clock that only moves when told to. Clones share the same time.
#[derive(Debug, Clone, Default)]
pub struct ManualClock {
    now: Arc<Mutex<Duration>>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&self, by: Duration) {
        *self.now.lock().unwrap() += by;
    }

    pub fn set(&self, to: Duration) {
        *self.now.lock().unwrap() = to;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_is_shared_between_clones() {
        let a = ManualClock::new();
        let b = a.clone();
        a.advance(Duration::from_millis(250));
        assert_eq!(b.now(), Duration::from_millis(250));
    }
}
