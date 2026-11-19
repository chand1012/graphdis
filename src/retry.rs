/// Capped exponential reconnect delay in seconds; no allocation or clock dependency.
pub(crate) struct Backoff {
    next_secs: u32,
}

impl Backoff {
    pub(crate) const fn new() -> Self {
        Self { next_secs: 1 }
    }

    pub(crate) fn next_secs(&mut self) -> u32 {
        let delay = self.next_secs;
        self.next_secs = (delay * 2).min(30);
        delay
    }

    pub(crate) fn reset(&mut self) {
        self.next_secs = 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_repeated_failures_and_resets_after_recovery() {
        let mut backoff = Backoff::new();
        for expected in [1, 2, 4, 8, 16, 30, 30, 30] {
            assert_eq!(backoff.next_secs(), expected);
        }
        for _ in 0..1000 {
            assert_eq!(backoff.next_secs(), 30);
        }
        backoff.reset();
        assert_eq!(backoff.next_secs(), 1);
        assert_eq!(backoff.next_secs(), 2);
    }
}
