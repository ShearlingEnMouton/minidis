use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
};

use crate::{
    command::{Command, Reply},
    log_err,
    store::Store,
};

#[derive(Clone)]
pub struct Aof {
    file: Arc<Mutex<File>>,
}

impl Aof {
    pub fn open(path: &str) -> io::Result<Aof> {
        if let Some(dir) = Path::new(path).parent() {
            fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Aof {
            file: Arc::new(Mutex::new(file)),
        })
    }

    pub fn append(file: &mut File, args: &[Vec<u8>]) -> io::Result<()> {
        let mut buf = Vec::new();
        Reply::Array(args.iter().cloned().map(Reply::Bulk).collect()).encode(&mut buf);
        file.write_all(&buf)
    }

    pub fn run_write(&self, cmd: Command, args: &[Vec<u8>], store: &Store) -> Reply {
        let mut file = self.file.lock().unwrap();
        let reply = cmd.execute(store);
        match &reply {
            Reply::Error(e) => {
                log_err!("Execution fail: {}", e);
            }
            _ => {
                if let Err(e) = Self::append(&mut file, args) {
                    log_err!("AOF append failed: {}", e);
                }
            }
        }
        reply
    }
}
