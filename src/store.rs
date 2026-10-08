use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Store {
    data: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Store {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.data.lock().unwrap().get(key).cloned()
    }
    pub fn set(&self, key: Vec<u8>, value: Vec<u8>) {
        self.data.lock().unwrap().insert(key, value);
    }
    pub fn del(&self, keys: &[Vec<u8>]) -> usize {
        let mut data = self.data.lock().unwrap();
        keys.iter()
            .filter(|key| data.remove(*key).is_some())
            .count()
    }
}
