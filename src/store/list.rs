use std::collections::VecDeque;

use super::{EntryType, Store, StoreError};

impl Store {
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
}
