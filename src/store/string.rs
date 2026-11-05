use super::{EntryType, Store, StoreError};

impl Store {
    fn parse_int(arg: &[u8]) -> Result<i64, StoreError> {
        let s = std::str::from_utf8(arg).map_err(|_| StoreError::NotAnInteger)?;
        s.parse().map_err(|_| StoreError::NotAnInteger)
    }

    pub fn exists(&self, keys: &[Vec<u8>]) -> usize {
        let mut g = self.inner.lock().unwrap();
        let mut cnt: usize = 0;
        for key in keys {
            g.purge_if_expired(key);

            if g.data.contains_key(key) {
                cnt += 1
            }
        }
        cnt
    }

    pub fn setnx(&self, key: Vec<u8>, value: Vec<u8>) -> bool {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(&key);

        if !g.data.contains_key(&key) {
            g.insert(key, EntryType::Value(value), None);
            return true;
        }

        false
    }

    pub fn mset(&self, pairs: Vec<(Vec<u8>, Vec<u8>)>) {
        let mut g = self.inner.lock().unwrap();
        for (k, v) in pairs {
            g.insert(k, EntryType::Value(v), None);
        }
    }

    pub fn mget(&self, keys: &[Vec<u8>]) -> Vec<Option<Vec<u8>>> {
        let mut g = self.inner.lock().unwrap();
        let mut res: Vec<Option<Vec<u8>>> = vec![];
        for key in keys {
            g.purge_if_expired(key);
            match g.data.get(key) {
                None => res.push(None),
                Some(e) => match &e.value {
                    EntryType::Value(v) => res.push(Some(v.to_vec())),
                    _ => res.push(None),
                },
            }
        }
        res
    }

    pub fn append(&self, key: Vec<u8>, value: Vec<u8>) -> Result<usize, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(&key);

        match g.data.get_mut(&key) {
            None => {
                let appended = value.len();
                g.insert(key, EntryType::Value(value), None);
                Ok(appended)
            }
            Some(e) => match &mut e.value {
                EntryType::Value(v) => {
                    v.extend_from_slice(&value);
                    Ok(v.len())
                }
                _ => Err(StoreError::WrongType),
            },
        }
    }

    pub fn incr_by(&self, key: Vec<u8>, delta: i64) -> Result<i64, StoreError> {
        let mut g = self.inner.lock().unwrap();
        g.purge_if_expired(&key);

        let added_delta = match g.data.get(&key) {
            None => delta,
            Some(e) => match &e.value {
                EntryType::Value(v) => {
                    let parsed = Self::parse_int(v)?;
                    parsed.checked_add(delta).ok_or(StoreError::Overflow)?
                }
                _ => return Err(StoreError::WrongType),
            },
        };

        if let Some(e) = g.data.get_mut(&key) {
            e.value = EntryType::Value(added_delta.to_string().into_bytes());
        } else {
            g.insert(
                key,
                EntryType::Value(added_delta.to_string().into_bytes()),
                None,
            );
        }

        Ok(added_delta)
    }
}
