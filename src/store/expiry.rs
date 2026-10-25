use std::time::{Duration, Instant};

use super::{Inner, Store};

pub enum TtlStatus {
    NoKey,
    NoExpiry,
    Remaining(Duration),
}

impl Store {
    pub fn ttl(&self, key: &[u8]) -> TtlStatus {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);
        match g.data.get(key) {
            None => TtlStatus::NoKey,
            Some(e) => match e.expires_at {
                None => TtlStatus::NoExpiry,
                Some(t) => TtlStatus::Remaining(t.saturating_duration_since(Instant::now())),
            },
        }
    }
    pub fn expire(&self, key: &[u8], ttl: Duration) -> u8 {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);
        g.set_expiry(key, Some(Instant::now() + ttl)) as u8
    }
    pub fn persist(&self, key: &[u8]) -> u8 {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);
        if g.data.get(key).is_some_and(|e| e.expires_at.is_some()) {
            g.set_expiry(key, None) as u8
        } else {
            0
        }
    }
    pub fn purge_expired(&self, limit: u64) -> u64 {
        self.inner.lock().unwrap().purge_expired(limit)
    }
}

impl Inner {
    fn set_expiry(&mut self, key: &[u8], new: Option<Instant>) -> bool {
        let Some(entry) = self.data.get_mut(key) else {
            return false;
        };
        if let Some(old) = entry.expires_at {
            self.expirations.remove(&(old, key.to_vec()));
        }
        entry.expires_at = new;
        if let Some(t) = new {
            self.expirations.insert((t, key.to_vec()));
        }
        true
    }
    fn purge_expired(&mut self, limit: u64) -> u64 {
        let now = Instant::now();
        let mut removed = 0;
        while removed < limit && self.expirations.first().is_some_and(|(t, _)| *t <= now) {
            let Some((_, key)) = self.expirations.pop_first() else {
                break;
            };
            self.data.remove(&key);
            removed += 1;
        }
        removed
    }
}
