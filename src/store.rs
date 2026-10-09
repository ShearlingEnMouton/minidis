use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Store {
    inner: Arc<Mutex<Inner>>,
}
pub enum StoreError {
    WrongType,
}
struct Inner {
    data: HashMap<Vec<u8>, Entry>,
    expirations: BTreeSet<(Instant, Vec<u8>)>,
}

enum EntryType {
    Value(Vec<u8>),
    List(VecDeque<Vec<u8>>),
    Hash(HashMap<Vec<u8>, Vec<u8>>),
}
struct Entry {
    value: EntryType,
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
            inner: Arc::new(Mutex::new(Inner {
                data: HashMap::new(),
                expirations: BTreeSet::new(),
            })),
        }
    }
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);
        match g.data.get(key) {
            None => Ok(None),
            Some(e) => match &e.value {
                EntryType::Value(v) => Ok(Some(v.clone())),
                _ => Err(StoreError::WrongType),
            },
        }
    }
    pub fn set(&self, key: Vec<u8>, value: Vec<u8>, ttl: Option<Duration>) {
        let expires_at = ttl.map(|d| Instant::now() + d);
        self.inner.lock().unwrap().insert(key, value, expires_at);
    }
    pub fn del(&self, keys: &[Vec<u8>]) -> usize {
        let mut g = self.inner.lock().unwrap();
        keys.iter()
            .filter(|k| g.remove(k).is_some_and(|e| !e.is_expired()))
            .count()
    }
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
    pub fn dbsize(&self) -> usize {
        let mut g = self.inner.lock().unwrap();
        g.dbsize()
    }
}

impl Inner {
    fn remove(&mut self, key: &[u8]) -> Option<Entry> {
        let entry = self.data.remove(key)?;
        if let Some(t) = entry.expires_at {
            self.expirations.remove(&(t, key.to_vec()));
        }
        Some(entry)
    }

    fn insert(&mut self, key: Vec<u8>, value: Vec<u8>, expires_at: Option<Instant>) {
        self.remove(&key);
        if let Some(t) = expires_at {
            self.expirations.insert((t, key.clone()));
        }
        self.data.insert(key, Entry { value: EntryType::Value(value), expires_at });
    }

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

    fn purge_if_expired(&mut self, key: &[u8]) {
        if self.data.get(key).is_some_and(|e| e.is_expired()) {
            self.remove(key);
        }
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

    fn dbsize(&mut self) -> usize {
        self.data.len()
    }
}
