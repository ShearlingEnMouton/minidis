use std::collections::HashMap;

use super::{EntryType, Store, StoreError};

pub type HgetallResult = Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError>;

impl Store {
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
}
