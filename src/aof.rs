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
    store::{SnapshotEntry, SnapshotValue, Store},
};

fn unix_now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn expire_at_ms(now_ms: u128, ttl: &Duration) -> Vec<u8> {
    (now_ms + ttl.as_millis())
        .min(i64::MAX as u128)
        .to_string()
        .into_bytes()
}

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
        let now_ms = unix_now_ms();
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
                expire_at_ms(now_ms, ttl),
            ],
            Command::Expire { key, ttl } => {
                vec![
                    b"PEXPIREAT".to_vec(),
                    key.clone(),
                    expire_at_ms(now_ms, ttl),
                ]
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
        let now_ms = unix_now_ms();

        let result = (|| -> io::Result<()> {
            for entry in &entries {
                Self::write_entry(&mut tmp, entry, now_ms)?;
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

    fn write_entry(file: &mut File, entry: &SnapshotEntry, now_ms: u128) -> io::Result<()> {
        let key = entry.key.clone();
        let expire_at = entry.ttl.as_ref().map(|ttl| expire_at_ms(now_ms, ttl));
        match &entry.value {
            SnapshotValue::Value(v) => {
                let mut args = vec![b"SET".to_vec(), key, v.clone()];
                if let Some(ms) = expire_at {
                    args.push(b"PXAT".to_vec());
                    args.push(ms);
                }
                Self::append(file, &args)
            }
            SnapshotValue::List(items) => {
                let mut args = vec![b"RPUSH".to_vec(), key.clone()];
                args.extend(items.iter().cloned());
                Self::append(file, &args)?;
                Self::append_expire(file, key, expire_at)
            }
            SnapshotValue::Hash(pairs) => {
                let mut args = vec![b"HSET".to_vec(), key.clone()];
                for (f, v) in pairs {
                    args.push(f.clone());
                    args.push(v.clone());
                }
                Self::append(file, &args)?;
                Self::append_expire(file, key, expire_at)
            }
        }
    }

    fn append_expire(file: &mut File, key: Vec<u8>, expire_at: Option<Vec<u8>>) -> io::Result<()> {
        match expire_at {
            Some(ms) => Self::append(file, &[b"PEXPIREAT".to_vec(), key, ms]),
            None => Ok(()),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TtlStatus;
    use std::sync::atomic::AtomicUsize;

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    const SET_A: &[u8] = b"*3\r\n$3\r\nSET\r\n$1\r\na\r\n$1\r\n1\r\n";

    struct TempAof {
        path: String,
    }

    impl TempAof {
        fn new() -> TempAof {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("minidis-aof-{}-{}.aof", std::process::id(), id))
                .to_string_lossy()
                .into_owned();
            TempAof { path }
        }
    }

    impl Drop for TempAof {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_file(format!("{}.tmp", self.path));
        }
    }

    fn args(parts: &[&str]) -> Vec<Vec<u8>> {
        parts.iter().map(|p| p.as_bytes().to_vec()).collect()
    }

    fn write(aof: &Aof, store: &Store, parts: &[&str]) -> Reply {
        let a = args(parts);
        let cmd = Command::from_args(&a).unwrap();
        aof.run_write(cmd, &a, store)
    }

    fn now_ms() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    }

    fn dump(store: &Store) -> Vec<String> {
        let mut lines: Vec<String> = store
            .snapshot()
            .into_iter()
            .map(|e| {
                let value = match e.value {
                    SnapshotValue::Value(v) => format!("string {:?}", v),
                    SnapshotValue::List(items) => format!("list {:?}", items),
                    SnapshotValue::Hash(mut pairs) => {
                        pairs.sort();
                        format!("hash {:?}", pairs)
                    }
                };
                format!("{:?} {} ttl={}", e.key, value, e.ttl.is_some())
            })
            .collect();
        lines.sort();
        lines
    }

    fn remaining(store: &Store, key: &str) -> Duration {
        match store.ttl(key.as_bytes()) {
            TtlStatus::Remaining(d) => d,
            _ => panic!("expected a TTL on {key}"),
        }
    }

    #[test]
    fn rewrite_then_replay_restores_every_type() {
        let tmp = TempAof::new();
        let aof = Aof::open(&tmp.path, FsyncPolicy::No).unwrap();
        let store = Store::new();
        store.set(b"s".to_vec(), b"1".to_vec(), Some(Duration::from_secs(100)));
        store.set(b"plain".to_vec(), b"2".to_vec(), None);
        store
            .rpush(b"l".to_vec(), vec![b"x".to_vec(), b"y".to_vec()])
            .unwrap();
        store.expire(b"l", Duration::from_secs(100));
        store
            .hset(
                b"h".to_vec(),
                vec![
                    (b"f1".to_vec(), b"v1".to_vec()),
                    (b"f2".to_vec(), b"v2".to_vec()),
                ],
            )
            .unwrap();

        aof.rewrite(&store).unwrap();

        let restored = Store::new();
        assert_eq!(Aof::replay(&tmp.path, &restored).unwrap(), 5);
        assert_eq!(dump(&restored), dump(&store));
        for key in ["s", "l"] {
            let left = remaining(&restored, key);
            assert!(left > Duration::from_secs(90) && left <= Duration::from_secs(100));
        }
    }

    #[test]
    fn rewrite_compacts_and_later_writes_go_to_the_new_file() {
        let tmp = TempAof::new();
        let aof = Aof::open(&tmp.path, FsyncPolicy::No).unwrap();
        let store = Store::new();
        write(&aof, &store, &["SET", "a", "1"]);
        write(&aof, &store, &["SET", "a", "2"]);
        write(&aof, &store, &["SET", "b", "1"]);
        let before = fs::metadata(&tmp.path).unwrap().len();

        aof.rewrite(&store).unwrap();

        let after = fs::metadata(&tmp.path).unwrap().len();
        assert!(after < before);
        assert!(!Path::new(&format!("{}.tmp", tmp.path)).exists());

        write(&aof, &store, &["SET", "c", "3"]);
        let restored = Store::new();
        assert_eq!(Aof::replay(&tmp.path, &restored).unwrap(), 3);
        assert_eq!(dump(&restored), dump(&store));
    }

    #[test]
    fn rewrite_leaves_an_empty_file_for_an_empty_store() {
        let tmp = TempAof::new();
        let aof = Aof::open(&tmp.path, FsyncPolicy::No).unwrap();
        let store = Store::new();
        write(&aof, &store, &["SET", "a", "1"]);
        write(&aof, &store, &["DEL", "a"]);

        aof.rewrite(&store).unwrap();

        assert_eq!(fs::metadata(&tmp.path).unwrap().len(), 0);
        assert_eq!(Aof::replay(&tmp.path, &Store::new()).unwrap(), 0);
    }

    #[test]
    fn failed_write_is_not_logged() {
        let tmp = TempAof::new();
        let aof = Aof::open(&tmp.path, FsyncPolicy::No).unwrap();
        let store = Store::new();
        write(&aof, &store, &["SET", "k", "v"]);

        let reply = write(&aof, &store, &["LPUSH", "k", "x"]);

        assert!(matches!(reply, Reply::Error(_)));
        assert_eq!(Aof::replay(&tmp.path, &Store::new()).unwrap(), 1);
    }

    #[test]
    fn replay_truncates_an_incomplete_tail() {
        let tmp = TempAof::new();
        let partial = b"*3\r\n$3\r\nSET\r\n$1\r\nb";
        fs::write(&tmp.path, [SET_A, &partial[..]].concat()).unwrap();
        let store = Store::new();

        assert_eq!(Aof::replay(&tmp.path, &store).unwrap(), 1);

        assert_eq!(fs::metadata(&tmp.path).unwrap().len(), SET_A.len() as u64);
        assert_eq!(store.get(b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(store.dbsize(), 1);
    }

    #[test]
    fn replay_fails_on_a_corrupt_record() {
        let tmp = TempAof::new();
        fs::write(&tmp.path, [SET_A, b"?garbage\r\n", SET_A].concat()).unwrap();

        let err = Aof::replay(&tmp.path, &Store::new()).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn replay_fails_on_an_unknown_command() {
        let tmp = TempAof::new();
        let mut file = File::create(&tmp.path).unwrap();
        Aof::append(&mut file, &args(&["NOPE", "a"])).unwrap();

        let err = Aof::replay(&tmp.path, &Store::new()).unwrap_err();

        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn replay_does_not_restore_a_key_whose_deadline_passed() {
        let tmp = TempAof::new();
        let mut file = File::create(&tmp.path).unwrap();
        Aof::append(&mut file, &args(&["SET", "gone", "v", "PXAT", "1000"])).unwrap();
        Aof::append(&mut file, &args(&["SET", "kept", "v"])).unwrap();
        let store = Store::new();

        Aof::replay(&tmp.path, &store).unwrap();

        assert_eq!(store.get(b"gone").unwrap(), None);
        assert_eq!(store.get(b"kept").unwrap(), Some(b"v".to_vec()));
        assert_eq!(store.dbsize(), 1);
    }

    #[test]
    fn replay_keeps_the_original_deadline() {
        let tmp = TempAof::new();
        let deadline = (now_ms() + 60_000).to_string();
        let mut file = File::create(&tmp.path).unwrap();
        Aof::append(&mut file, &args(&["SET", "k", "v", "PXAT", &deadline])).unwrap();
        let store = Store::new();

        Aof::replay(&tmp.path, &store).unwrap();

        let left = remaining(&store, "k");
        assert!(left > Duration::from_secs(55) && left <= Duration::from_secs(60));
    }

    #[test]
    fn convert_args_turns_a_set_ttl_into_pxat() {
        let a = args(&["SET", "k", "v", "EX", "60"]);
        let cmd = Command::from_args(&a).unwrap();

        let before = now_ms();
        let out = Aof::convert_args(&cmd, &a);
        let after = now_ms();

        assert_eq!(out.len(), 5);
        assert_eq!(out[..4], args(&["SET", "k", "v", "PXAT"])[..]);
        let ms: u128 = String::from_utf8(out[4].clone()).unwrap().parse().unwrap();
        assert!(ms >= before + 60_000 && ms <= after + 60_000);
    }

    #[test]
    fn convert_args_turns_expire_into_pexpireat() {
        let a = args(&["EXPIRE", "k", "30"]);
        let cmd = Command::from_args(&a).unwrap();

        let before = now_ms();
        let out = Aof::convert_args(&cmd, &a);
        let after = now_ms();

        assert_eq!(out.len(), 3);
        assert_eq!(out[..2], args(&["PEXPIREAT", "k"])[..]);
        let ms: u128 = String::from_utf8(out[2].clone()).unwrap().parse().unwrap();
        assert!(ms >= before + 30_000 && ms <= after + 30_000);
    }

    #[test]
    fn convert_args_leaves_other_commands_unchanged() {
        for parts in [
            &["SET", "k", "v"][..],
            &["DEL", "a", "b"][..],
            &["LPUSH", "l", "x"][..],
            &["HSET", "h", "f", "v"][..],
            &["PERSIST", "k"][..],
        ] {
            let a = args(parts);
            let cmd = Command::from_args(&a).unwrap();
            assert_eq!(Aof::convert_args(&cmd, &a), a);
        }
    }

    #[test]
    fn run_write_follows_the_fsync_policy() {
        for (policy, expect_dirty) in [
            (FsyncPolicy::Always, false),
            (FsyncPolicy::EverySec, true),
            (FsyncPolicy::No, false),
        ] {
            let tmp = TempAof::new();
            let aof = Aof::open(&tmp.path, policy).unwrap();
            let store = Store::new();

            write(&aof, &store, &["SET", "k", "v"]);

            assert_eq!(aof.dirty.load(Ordering::Relaxed), expect_dirty);
            assert!(fs::metadata(&tmp.path).unwrap().len() > 0);
        }
    }

    #[test]
    fn everysec_task_clears_the_dirty_flag() {
        let tmp = TempAof::new();
        let aof = Aof::open(&tmp.path, FsyncPolicy::EverySec).unwrap();
        aof.start_fsync_task();
        let store = Store::new();

        write(&aof, &store, &["SET", "k", "v"]);
        assert!(aof.dirty.load(Ordering::Relaxed));

        std::thread::sleep(Duration::from_millis(1600));
        assert!(!aof.dirty.load(Ordering::Relaxed));
    }
}
