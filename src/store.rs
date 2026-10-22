use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Store {
    inner: Arc<Mutex<Inner>>,
}
#[derive(Debug)]
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

pub type HgetallResult = Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError>;

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
        self.inner
            .lock()
            .unwrap()
            .insert(key, EntryType::Value(value), expires_at);
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

    pub fn lpush(&self, key: Vec<u8>, values: Vec<Vec<u8>>) -> Result<usize, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(&key);

        match g.data.get_mut(&key) {
            None => {
                let len = values.len();
                let rev_value: VecDeque<Vec<u8>> = values.into_iter().rev().collect();
                g.insert(key, EntryType::List(rev_value), None);
                Ok(len)
            }
            Some(e) => match &mut e.value {
                EntryType::List(list) => {
                    for v in values {
                        list.push_front(v)
                    }
                    Ok(list.len())
                }
                _ => Err(StoreError::WrongType),
            },
        }
    }
    pub fn rpush(&self, key: Vec<u8>, values: Vec<Vec<u8>>) -> Result<usize, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(&key);

        match g.data.get_mut(&key) {
            None => {
                let len = values.len();
                let value: VecDeque<Vec<u8>> = values.into_iter().collect();
                g.insert(key, EntryType::List(value), None);
                Ok(len)
            }
            Some(e) => match &mut e.value {
                EntryType::List(list) => {
                    list.extend(values);
                    Ok(list.len())
                }
                _ => Err(StoreError::WrongType),
            },
        }
    }
    pub fn lpop(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);

        match g.data.get_mut(key) {
            None => Ok(None),
            Some(e) => {
                let (removed, now_empty) = match &mut e.value {
                    EntryType::List(list) => {
                        let v = list.pop_front();
                        (v, list.is_empty())
                    }
                    _ => return Err(StoreError::WrongType),
                };
                if now_empty {
                    g.remove(key);
                }
                Ok(removed)
            }
        }
    }
    pub fn rpop(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);

        match g.data.get_mut(key) {
            None => Ok(None),
            Some(e) => {
                let (removed, now_empty) = match &mut e.value {
                    EntryType::List(list) => {
                        let v = list.pop_back();
                        (v, list.is_empty())
                    }
                    _ => return Err(StoreError::WrongType),
                };
                if now_empty {
                    g.remove(key);
                }
                Ok(removed)
            }
        }
    }
    pub fn llen(&self, key: &[u8]) -> Result<usize, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);

        match g.data.get(key) {
            None => Ok(0),
            Some(e) => match &e.value {
                EntryType::List(list) => Ok(list.len()),
                _ => Err(StoreError::WrongType),
            },
        }
    }
    pub fn lrange(&self, key: &[u8], start: i64, stop: i64) -> Result<Vec<Vec<u8>>, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);

        match g.data.get(key) {
            None => Ok(Vec::new()),
            Some(e) => match &e.value {
                EntryType::List(list) => {
                    let len = list.len() as i64;
                    let start = if start < 0 {
                        (len + start).max(0)
                    } else {
                        start
                    };
                    let stop = if stop < 0 {
                        len + stop
                    } else {
                        stop.min(len - 1)
                    };
                    if start > stop || start >= len {
                        return Ok(Vec::new());
                    }
                    Ok(list
                        .range(start as usize..=stop as usize)
                        .cloned()
                        .collect())
                }
                _ => Err(StoreError::WrongType),
            },
        }
    }

    pub fn hset(&self, key: Vec<u8>, pairs: Vec<(Vec<u8>, Vec<u8>)>) -> Result<usize, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(&key);

        match g.data.get_mut(&key) {
            None => {
                let hash: HashMap<_, _> = pairs.into_iter().collect();
                let added = hash.len();
                g.insert(key, EntryType::Hash(hash), None);
                Ok(added)
            }
            Some(e) => match &mut e.value {
                EntryType::Hash(hash) => Ok(pairs
                    .into_iter()
                    .map(|(f, v)| hash.insert(f, v).is_none())
                    .filter(|&new| new)
                    .count()),
                _ => Err(StoreError::WrongType),
            },
        }
    }
    pub fn hget(&self, key: &[u8], field: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);

        match g.data.get(key) {
            None => Ok(None),
            Some(e) => match &e.value {
                EntryType::Hash(hash) => Ok(hash.get(field).cloned()),
                _ => Err(StoreError::WrongType),
            },
        }
    }
    pub fn hdel(&self, key: &[u8], fields: &[Vec<u8>]) -> Result<usize, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);

        match g.data.get_mut(key) {
            None => Ok(0),
            Some(e) => {
                let (removed, now_empty) = match &mut e.value {
                    EntryType::Hash(hash) => {
                        let n = fields.iter().filter(|f| hash.remove(*f).is_some()).count();
                        (n, hash.is_empty())
                    }
                    _ => return Err(StoreError::WrongType),
                };
                if now_empty {
                    g.remove(key);
                }
                Ok(removed)
            }
        }
    }
    pub fn hlen(&self, key: &[u8]) -> Result<usize, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);

        match g.data.get(key) {
            None => Ok(0),
            Some(e) => match &e.value {
                EntryType::Hash(hash) => Ok(hash.len()),
                _ => Err(StoreError::WrongType),
            },
        }
    }
    pub fn hexists(&self, key: &[u8], field: &[u8]) -> Result<bool, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);

        match g.data.get(key) {
            None => Ok(false),
            Some(e) => match &e.value {
                EntryType::Hash(hash) => Ok(hash.contains_key(field)),
                _ => Err(StoreError::WrongType),
            },
        }
    }
    pub fn hgetall(&self, key: &[u8]) -> HgetallResult {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);

        match g.data.get(key) {
            None => Ok(Vec::new()),
            Some(e) => match &e.value {
                EntryType::Hash(hash) => {
                    Ok(hash.iter().map(|(f, v)| (f.clone(), v.clone())).collect())
                }
                _ => Err(StoreError::WrongType),
            },
        }
    }

    pub fn key_type(&self, key: &[u8]) -> &'static str {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(key);
        match g.data.get(key).map(|e| &e.value) {
            None => "none",
            Some(EntryType::Value(_)) => "string",
            Some(EntryType::List(_)) => "list",
            Some(EntryType::Hash(_)) => "hash",
        }
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

    fn insert(&mut self, key: Vec<u8>, value: EntryType, expires_at: Option<Instant>) {
        self.remove(&key);
        if let Some(t) = expires_at {
            self.expirations.insert((t, key.clone()));
        }
        self.data.insert(key, Entry { value, expires_at });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    fn check_invariants(store: &Store) {
        let g = store.inner.lock().unwrap();
        let with_ttl = g.data.values().filter(|e| e.expires_at.is_some()).count();
        assert_eq!(with_ttl, g.expirations.len());
        for (t, key) in &g.expirations {
            let entry = g.data.get(key).expect("indexed key missing from data");
            assert_eq!(entry.expires_at, Some(*t));
        }
        for entry in g.data.values() {
            match &entry.value {
                EntryType::List(l) => assert!(!l.is_empty(), "empty list kept"),
                EntryType::Hash(h) => assert!(!h.is_empty(), "empty hash kept"),
                EntryType::Value(_) => {}
            }
        }
    }

    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) % n
        }
    }

    #[test]
    fn invariants_hold_after_random_ops() {
        let store = Store::new();
        let mut rng = Lcg(42);
        let keys = ["a", "b", "c", "d"];
        for i in 0..5000 {
            let key = k(keys[rng.next(keys.len() as u64) as usize]);
            let ttl = Duration::from_millis(rng.next(5));
            match rng.next(12) {
                0 => store.set(key, k("v"), None),
                1 => store.set(key, k("v"), Some(ttl)),
                2 => {
                    store.expire(&key, ttl);
                }
                3 => {
                    store.persist(&key);
                }
                4 => {
                    store.del(&[key]);
                }
                5 => {
                    let _ = store.lpush(key, vec![k("x")]);
                }
                6 => {
                    let _ = store.rpush(key, vec![k("x"), k("y")]);
                }
                7 => {
                    let _ = store.lpop(&key);
                }
                8 => {
                    let _ = store.rpop(&key);
                }
                9 => {
                    let _ = store.hset(key, vec![(k("f"), k("1"))]);
                }
                10 => {
                    let _ = store.hdel(&key, &[k("f")]);
                }
                _ => {
                    store.purge_expired(2);
                }
            }
            check_invariants(&store);
            if i % 500 == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    #[test]
    fn dbsize_counts_every_type_and_drops_empty_collections() {
        let store = Store::new();
        store.set(k("s"), k("v"), None);
        store.lpush(k("l"), vec![k("a")]).unwrap();
        store.hset(k("h"), vec![(k("f"), k("v"))]).unwrap();
        assert_eq!(store.dbsize(), 3);

        assert_eq!(store.lpop(&k("l")).unwrap(), Some(k("a")));
        assert_eq!(store.hdel(&k("h"), &[k("f")]).unwrap(), 1);
        assert_eq!(store.dbsize(), 1);
        assert_eq!(store.key_type(&k("l")), "none");
        assert_eq!(store.key_type(&k("h")), "none");
        check_invariants(&store);
    }

    #[test]
    fn key_type_reports_each_type() {
        let store = Store::new();
        store.set(k("s"), k("v"), None);
        store.rpush(k("l"), vec![k("a")]).unwrap();
        store.hset(k("h"), vec![(k("f"), k("v"))]).unwrap();
        assert_eq!(store.key_type(&k("s")), "string");
        assert_eq!(store.key_type(&k("l")), "list");
        assert_eq!(store.key_type(&k("h")), "hash");
        assert_eq!(store.key_type(&k("missing")), "none");
    }

    #[test]
    fn wrong_type_is_rejected_without_changes() {
        let store = Store::new();
        store.set(k("s"), k("v"), None);
        store.lpush(k("l"), vec![k("a")]).unwrap();
        assert!(matches!(store.get(&k("l")), Err(StoreError::WrongType)));
        assert!(matches!(
            store.lpush(k("s"), vec![k("a")]),
            Err(StoreError::WrongType)
        ));
        assert!(matches!(
            store.hget(&k("l"), &k("f")),
            Err(StoreError::WrongType)
        ));
        assert!(matches!(
            store.hset(k("l"), vec![(k("f"), k("v"))]),
            Err(StoreError::WrongType)
        ));
        assert_eq!(store.get(&k("s")).unwrap(), Some(k("v")));
        assert_eq!(store.llen(&k("l")).unwrap(), 1);
    }

    #[test]
    fn push_order_and_lrange_bounds() {
        let store = Store::new();
        store.rpush(k("l"), vec![k("a"), k("b")]).unwrap();
        store.lpush(k("l"), vec![k("x"), k("y")]).unwrap();
        let all = vec![k("y"), k("x"), k("a"), k("b")];
        assert_eq!(store.lrange(&k("l"), 0, -1).unwrap(), all);
        assert_eq!(
            store.lrange(&k("l"), -2, 100).unwrap(),
            vec![k("a"), k("b")]
        );
        assert_eq!(store.lrange(&k("l"), -100, 0).unwrap(), vec![k("y")]);
        assert_eq!(store.lrange(&k("l"), 1, 1).unwrap(), vec![k("x")]);
        assert!(store.lrange(&k("l"), 3, 1).unwrap().is_empty());
        assert!(store.lrange(&k("l"), 10, 20).unwrap().is_empty());
        assert!(store.lrange(&k("l"), 0, -100).unwrap().is_empty());
        assert!(store.lrange(&k("missing"), 0, -1).unwrap().is_empty());
    }

    #[test]
    fn hset_counts_only_new_fields() {
        let store = Store::new();
        let pairs = vec![(k("f"), k("1")), (k("f"), k("2"))];
        assert_eq!(store.hset(k("h"), pairs).unwrap(), 1);
        assert_eq!(store.hget(&k("h"), &k("f")).unwrap(), Some(k("2")));
        let pairs = vec![(k("f"), k("3")), (k("g"), k("4"))];
        assert_eq!(store.hset(k("h"), pairs).unwrap(), 1);
        assert_eq!(store.hlen(&k("h")).unwrap(), 2);
        assert_eq!(store.hget(&k("h"), &k("f")).unwrap(), Some(k("3")));
    }

    #[test]
    fn writes_to_a_collection_keep_its_ttl() {
        let store = Store::new();
        store.rpush(k("l"), vec![k("a")]).unwrap();
        assert!(matches!(store.ttl(&k("l")), TtlStatus::NoExpiry));
        store.expire(&k("l"), Duration::from_secs(100));
        store.rpush(k("l"), vec![k("b")]).unwrap();
        store.lpop(&k("l")).unwrap();
        assert!(matches!(store.ttl(&k("l")), TtlStatus::Remaining(_)));

        store.hset(k("h"), vec![(k("f"), k("1"))]).unwrap();
        store.expire(&k("h"), Duration::from_secs(100));
        store.hset(k("h"), vec![(k("g"), k("2"))]).unwrap();
        assert!(matches!(store.ttl(&k("h")), TtlStatus::Remaining(_)));
        check_invariants(&store);
    }

    #[test]
    fn set_overwrites_any_type_and_clears_ttl() {
        let store = Store::new();
        store.lpush(k("x"), vec![k("a")]).unwrap();
        store.expire(&k("x"), Duration::from_secs(100));
        store.set(k("x"), k("v"), None);
        assert_eq!(store.key_type(&k("x")), "string");
        assert!(matches!(store.ttl(&k("x")), TtlStatus::NoExpiry));
        check_invariants(&store);
    }

    #[test]
    fn expired_collection_is_recreated_without_ttl() {
        let store = Store::new();
        store.rpush(k("l"), vec![k("old")]).unwrap();
        store.expire(&k("l"), Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(store.rpush(k("l"), vec![k("new")]).unwrap(), 1);
        assert_eq!(store.lrange(&k("l"), 0, -1).unwrap(), vec![k("new")]);
        assert!(matches!(store.ttl(&k("l")), TtlStatus::NoExpiry));
        check_invariants(&store);
    }
}
