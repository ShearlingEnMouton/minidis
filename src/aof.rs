use std::{
    fs::{self, File, OpenOptions},
    io::{self, ErrorKind, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    command::{Command, Reply},
    log_err, log_warn, resp,
    store::{SnapshotValue, Store},
};

#[derive(Clone)]
pub struct Aof {
    file: Arc<Mutex<File>>,
    path: String,
    policy: FsyncPolicy,
    dirty: Arc<AtomicBool>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum FsyncPolicy {
    Always,
    EverySec,
    No,
}

impl Aof {
    pub fn open(path: &str, policy: FsyncPolicy) -> io::Result<Aof> {
        if let Some(dir) = Path::new(path).parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Aof {
            file: Arc::new(Mutex::new(file)),
            path: path.to_string(),
            policy,
            dirty: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn append(file: &mut File, args: &[Vec<u8>]) -> io::Result<()> {
        let mut buf = Vec::new();
        Reply::Array(args.iter().cloned().map(Reply::Bulk).collect()).encode(&mut buf);
        file.write_all(&buf)
    }

    fn convert_args(cmd: &Command, args: &[Vec<u8>]) -> Vec<Vec<u8>> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let expire_at_ms = |ttl: &Duration| {
            (now_ms + ttl.as_millis())
                .min(i64::MAX as u128)
                .to_string()
                .into_bytes()
        };
        match cmd {
            Command::Set {
                key,
                value,
                ttl: Some(ttl),
            } => vec![
                b"SET".to_vec(),
                key.clone(),
                value.clone(),
                b"PXAT".to_vec(),
                expire_at_ms(ttl),
            ],
            Command::Expire { key, ttl } => {
                vec![b"PEXPIREAT".to_vec(), key.clone(), expire_at_ms(ttl)]
            }
            _ => args.to_vec(),
        }
    }

    pub fn run_write(&self, cmd: Command, args: &[Vec<u8>], store: &Store) -> Reply {
        let policy = self.policy;
        let mut file = self.file.lock().unwrap();
        let aof_args = Self::convert_args(&cmd, args);
        let reply = cmd.execute(store);
        match &reply {
            Reply::Error(e) => {
                log_err!("Execution fail: {}", e);
            }
            _ => {
                if let Err(e) = Self::append(&mut file, &aof_args) {
                    log_err!("AOF append failed: {}", e);
                } else {
                    match policy {
                        FsyncPolicy::Always => {
                            if let Err(e) = file.sync_data() {
                                log_err!("Sync data failed: {}", e);
                            }
                        }
                        FsyncPolicy::EverySec => self.dirty.store(true, Ordering::Relaxed),
                        FsyncPolicy::No => {}
                    }
                }
            }
        }
        reply
    }

    pub fn rewrite(&self, store: &Store) -> io::Result<()> {
        let mut file = self.file.lock().unwrap();
        let entries = store.snapshot();
        let tmp_path = format!("{}.tmp", self.path);
        match fs::remove_file(&tmp_path) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let mut tmp = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&tmp_path)?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        let result = (|| -> io::Result<()> {
            for entry in &entries {
                let key = entry.key.clone();
                let expire_at = entry.ttl.as_ref().map(|ttl| {
                    (now_ms + ttl.as_millis())
                        .min(i64::MAX as u128)
                        .to_string()
                        .into_bytes()
                });
                match &entry.value {
                    SnapshotValue::Value(v) => {
                        let mut args = vec![b"SET".to_vec(), key, v.clone()];
                        if let Some(ms) = expire_at {
                            args.push(b"PXAT".to_vec());
                            args.push(ms);
                        }
                        Self::append(&mut tmp, &args)?;
                    }
                    SnapshotValue::List(items) => {
                        let mut args = vec![b"RPUSH".to_vec(), key.clone()];
                        args.extend(items.iter().cloned());
                        Self::append(&mut tmp, &args)?;
                        if let Some(ms) = expire_at {
                            Self::append(&mut tmp, &[b"PEXPIREAT".to_vec(), key, ms])?;
                        }
                    }
                    SnapshotValue::Hash(pairs) => {
                        let mut args = vec![b"HSET".to_vec(), key.clone()];
                        for (f, v) in pairs {
                            args.push(f.clone());
                            args.push(v.clone());
                        }
                        Self::append(&mut tmp, &args)?;
                        if let Some(ms) = expire_at {
                            Self::append(&mut tmp, &[b"PEXPIREAT".to_vec(), key, ms])?;
                        }
                    }
                }
            }
            tmp.sync_all()?;
            fs::rename(&tmp_path, &self.path)
        })();

        if let Err(e) = result {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }
        *file = tmp;
        Ok(())
    }

    pub fn replay(path: &str, store: &Store) -> io::Result<usize> {
        let data = fs::read(path)?;
        let mut offset = 0;
        let mut count = 0;
        while offset < data.len() {
            match resp::parse_command(&data[offset..]) {
                Ok(Some((args, consumed))) => {
                    if !args.is_empty() {
                        let cmd = Command::from_args(&args).map_err(|_| {
                            io::Error::new(
                                ErrorKind::InvalidData,
                                format!("invalid command in AOF at byte {}", offset),
                            )
                        })?;
                        cmd.execute(store);
                        count += 1;
                    }
                    offset += consumed;
                }
                Ok(None) => {
                    log_warn!(
                        "AOF has an incomplete record at byte {}, truncating",
                        offset
                    );
                    OpenOptions::new()
                        .write(true)
                        .open(path)?
                        .set_len(offset as u64)?;
                    break;
                }
                Err(e) => {
                    return Err(io::Error::new(
                        ErrorKind::InvalidData,
                        format!("corrupted AOF at byte {}: {:?}", offset, e),
                    ));
                }
            }
        }
        Ok(count)
    }

    pub fn start_fsync_task(&self) {
        if self.policy != FsyncPolicy::EverySec {
            return;
        }
        let aof_file = self.file.clone();
        let is_dirty = self.dirty.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if is_dirty.swap(false, Ordering::Relaxed)
                    && let Err(e) = aof_file.lock().unwrap().sync_data()
                {
                    log_err!("AOF fsync failed: {}", e);
                }
            }
        });
    }
}
