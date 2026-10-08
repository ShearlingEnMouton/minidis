use std::time::{Duration, Instant};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Store {
    data: Arc<Mutex<HashMap<Vec<u8>, Entry>>>,
}

struct Entry {
    value: Vec<u8>,
    expires_at: Option<Instant>,
}

impl Entry {
    fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|t| Instant::now() >= t)
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

pub enum TtlStatus {
    NoKey,
    NoExpiry,
    Remaining(Duration),
}

impl Store {
    pub fn new() -> Store {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        let mut data = self.data.lock().unwrap();
        if data.get(key).is_some_and(|e| e.is_expired()) {
            data.remove(key);
        }
        data.get(key).map(|e| e.value.clone())
    }
    pub fn set(&self, key: Vec<u8>, value: Vec<u8>, ttl: Option<Duration>) {
        let expires_at = ttl.map(|d| Instant::now() + d);
        self.data
            .lock()
            .unwrap()
            .insert(key, Entry { value, expires_at });
    }
    pub fn del(&self, keys: &[Vec<u8>]) -> usize {
        let mut data = self.data.lock().unwrap();
        let mut removed = 0;
        for key in keys {
            if let Some(entry) = data.remove(key)
                && !entry.is_expired()
            {
                removed += 1;
            }
        }
        removed
    }
    pub fn ttl(&self, key: &[u8]) -> TtlStatus {
        let mut data = self.data.lock().unwrap();
        if data.get(key).is_some_and(|e| e.is_expired()) {
            data.remove(key);
        }
        match data.get(key) {
            None => TtlStatus::NoKey,
            Some(e) => match e.expires_at {
                None => TtlStatus::NoExpiry,
                Some(t) => TtlStatus::Remaining(t.saturating_duration_since(Instant::now())),
            },
        }
    }
}
