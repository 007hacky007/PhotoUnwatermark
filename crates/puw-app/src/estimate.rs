//! Progress and time estimates.

/// How far along a model load is, from how long it took last time.
/// Never reaches 1.0, because the load is not done until it says so.
pub fn load_fraction(elapsed: f32, expected: f32) -> f32 {
    if expected <= 0.0 {
        return 0.0;
    }
    (elapsed / expected).clamp(0.0, 0.95)
}

/// Time-left estimate for a batch of photos.
#[derive(Debug, Clone)]
pub struct Eta {
    total: usize,
    done: usize,
    spent: f32,
}

impl Eta {
    pub fn new(total: usize) -> Eta {
        Eta {
            total,
            done: 0,
            spent: 0.0,
        }
    }

    /// Record that one photo took `seconds`.
    pub fn photo_done(&mut self, seconds: f32) {
        self.done += 1;
        self.spent += seconds;
    }

    /// Record that one photo was skipped at once (it could not be read), so it neither
    /// counts toward the time per photo nor stays among the photos left.
    pub fn photo_skipped(&mut self) {
        self.total = self.total.saturating_sub(1);
    }

    /// Seconds left, or `None` until the first photo has finished.
    /// `current_elapsed` is how long the photo in progress has already taken.
    pub fn seconds_left(&self, current_elapsed: f32) -> Option<u32> {
        if self.done == 0 {
            return None;
        }
        let mean = self.spent / self.done as f32;
        let remaining = self.total.saturating_sub(self.done) as f32;
        Some((mean * remaining - current_elapsed).max(0.0).round() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_fraction_caps() {
        assert_eq!(load_fraction(5.0, 10.0), 0.5);
        assert_eq!(load_fraction(30.0, 10.0), 0.95);
        assert_eq!(load_fraction(1.0, 0.0), 0.0);
        assert_eq!(load_fraction(1.0, -2.0), 0.0);
        assert_eq!(load_fraction(-1.0, 10.0), 0.0);
    }

    #[test]
    fn eta_none_before_first() {
        assert_eq!(Eta::new(5).seconds_left(3.0), None);
    }

    #[test]
    fn eta_mean_times_remaining() {
        let mut e = Eta::new(5);
        e.photo_done(10.0);
        e.photo_done(20.0);
        // mean 15, 3 remaining, 5 already spent on the current one
        assert_eq!(e.seconds_left(5.0), Some(40));
        assert_eq!(e.seconds_left(100.0), Some(0));
    }

    #[test]
    fn skipped_photo_does_not_count() {
        // The first photo was missing (skipped at once): no time is known yet.
        let mut e = Eta::new(4);
        e.photo_skipped();
        assert_eq!(e.seconds_left(0.0), None);
        e.photo_done(10.0);
        // Two photos are left at 10 s each, not two at 5 s.
        assert_eq!(e.seconds_left(0.0), Some(20));
    }

    #[test]
    fn eta_when_all_done_is_zero() {
        let mut e = Eta::new(1);
        e.photo_done(3.0);
        assert_eq!(e.seconds_left(0.0), Some(0));
    }
}
