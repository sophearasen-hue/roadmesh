//! Rate limits (AUDIT S-18, CLAUDE.md "not flooding"): a token bucket per session for
//! publishes and one per source address for enrolment attempts.

use std::collections::HashMap;
use std::hash::Hash;

/// Tokens refill at `rate` per second up to `burst`.
#[derive(Debug, Clone, Copy)]
pub struct TokenBucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    at: f64,
}

impl TokenBucket {
    /// A full bucket at time `now` (s).
    pub fn new(rate: f64, burst: f64, now: f64) -> Self {
        Self {
            rate,
            burst,
            tokens: burst,
            at: now,
        }
    }

    /// Take one token at `now`; false when empty.
    pub fn take(&mut self, now: f64) -> bool {
        let dt = (now - self.at).max(0.0);
        self.at = now.max(self.at);
        self.tokens = (self.tokens + dt * self.rate).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// One bucket per key (e.g. source IP), forgetting idle keys.
#[derive(Debug, Clone)]
pub struct KeyedLimiter<K> {
    rate: f64,
    burst: f64,
    buckets: HashMap<K, TokenBucket>,
    max_keys: usize,
}

impl<K: Eq + Hash + Clone> KeyedLimiter<K> {
    /// `max_keys` caps memory: when full, idle buckets are dropped first; if none are
    /// idle the request is refused.
    pub fn new(rate: f64, burst: f64, max_keys: usize) -> Self {
        Self {
            rate,
            burst,
            buckets: HashMap::new(),
            max_keys,
        }
    }

    /// Take one token for `key` at `now`.
    pub fn take(&mut self, key: &K, now: f64) -> bool {
        if !self.buckets.contains_key(key) && self.buckets.len() >= self.max_keys {
            let full = self.burst / self.rate;
            self.buckets.retain(|_, b| now - b.at < full);
            if self.buckets.len() >= self.max_keys {
                return false;
            }
        }
        self.buckets
            .entry(key.clone())
            .or_insert_with(|| TokenBucket::new(self.rate, self.burst, now))
            .take(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_allows_burst_then_rate() {
        let mut b = TokenBucket::new(10.0, 20.0, 0.0);
        assert_eq!((0..25).filter(|_| b.take(0.0)).count(), 20);
        assert!(!b.take(0.05));
        assert!(b.take(0.1));
        // one second later: 10 more
        assert_eq!((0..15).filter(|_| b.take(1.1)).count(), 10);
    }

    #[test]
    fn keyed_limiter_is_per_key_and_bounded() {
        let mut l = KeyedLimiter::new(1.0, 2.0, 2);
        assert!(l.take(&"a", 0.0) && l.take(&"a", 0.0) && !l.take(&"a", 0.0));
        assert!(l.take(&"b", 0.0));
        assert!(!l.take(&"c", 0.0)); // full, nobody idle
        assert!(l.take(&"c", 10.0)); // a and b idle now
    }
}
