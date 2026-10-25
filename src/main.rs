use std::{net::SocketAddr, time::Duration};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task,
    time::{MissedTickBehavior, interval},
};

use crate::aof::{Aof, FsyncPolicy};
use crate::command::{Command, Reply};
use crate::store::Store;
mod aof;
mod command;
mod limits;
mod logger;
mod resp;
mod store;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = std::env::var("EXPOSE_PORT").unwrap_or_else(|_| "3131".to_string());
    let addr = format!("{}:{}", bind_addr, port);
    let listener = TcpListener::bind(addr.clone()).await?;
    log_info!("Listening on: {}", addr);

    let purge_limit: u64 = match std::env::var("PURGE_LIMIT") {
        Ok(v) => v.parse().expect("PURGE_LIMIT must be a positive integer"),
        Err(_) => 1000,
    };

    let purge_int: u64 = match std::env::var("PURGE_INTERVAL") {
        Ok(v) => v
            .parse()
            .expect("PURGE_INTERVAL must be a positive integer"),
        Err(_) => 100,
    };

    if purge_limit == 0 || purge_int == 0 {
        log_err!("purge_limit and purge_int should be a positive value");
        return Err("purge_limit and purge_int should be a positive value".into());
    }

    let aof_path =
        std::env::var("AOF_PATH").unwrap_or_else(|_| "/data/appendonly.aof".to_string());

    let fsync = std::env::var("AOF_FSYNC").unwrap_or_else(|_| "everysec".to_string());
    let policy = match fsync.to_ascii_lowercase().as_str() {
        "always" => FsyncPolicy::Always,
        "everysec" => FsyncPolicy::EverySec,
        "no" => FsyncPolicy::No,
        _ => return Err("AOF_FSYNC must be always, everysec or no".into()),
    };

    let store = Store::new();
    let aof = Aof::open(&aof_path, policy)?;
    log_info!("AOF file: {}", aof_path);
    let replayed = Aof::replay(&aof_path, &store)?;
    log_info!("AOF replayed: {} commands", replayed);

    aof.start_fsync_task();

    let reaper_store = store.clone();
    task::spawn(async move {
        let mut purge_interval = interval(Duration::from_millis(purge_int));
        purge_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            let purged = reaper_store.purge_expired(purge_limit);
            if purged > 0 {
                log_debug!("{purged} expired data have been purged.");
            }
            if purged == purge_limit {
                task::yield_now().await;
                continue;
            }
            purge_interval.tick().await;
        }
    });

    loop {
        let (socket, peer_addr) = listener.accept().await?;
        let store = store.clone();
        let aof = aof.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, peer_addr, &store, &aof).await {
                log_err!("Error handling client {}: {:?}", peer_addr, e);
            }
        });
    }
}

async fn handle_client(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    store: &Store,
    aof: &Aof,
) -> tokio::io::Result<()> {
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let mut seen_data = false;

    loop {
        let bytes_read = stream.read_buf(&mut buffer).await?;

        if bytes_read == 0 {
            if seen_data {
                log_info!("Client disconnected: {}", peer_addr);
            } else {
                log_debug!("Connection closed without data: {}", peer_addr);
            }
            return Ok(());
        }

        if !seen_data {
            seen_data = true;
            log_info!("New client connected: {}", peer_addr);
        }

        loop {
            match resp::parse_command(&buffer) {
                Ok(Some((args, consumed))) => {
                    buffer.drain(..consumed);
                    if args.is_empty() {
                        continue;
                    }
                    let mut out = Vec::new();

                    match Command::from_args(&args) {
                        Ok(cmd) => {
                            if matches!(cmd, Command::Rewriteaof) {
                                match aof.rewrite(store) {
                                    Ok(()) => {
                                        log_info!("AOF rewritten");
                                        Reply::Simple("OK").encode(&mut out);
                                    }
                                    Err(e) => {
                                        log_err!("AOF rewrite failed: {}", e);
                                        Reply::Error(format!("ERR AOF rewrite failed: {e}"))
                                            .encode(&mut out);
                                    }
                                }
                            } else if cmd.is_write() {
                                aof.run_write(cmd, &args, store).encode(&mut out);
                            } else {
                                cmd.execute(store).encode(&mut out);
                            }
                            stream.write_all(&out).await?;
                        }
                        Err(e) => {
                            Reply::from(e).encode(&mut out);
                            stream.write_all(&out).await?
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    log_err!("Protocol error: {:?}", e);
                    stream.write_all(b"-ERR Protocol error\r\n").await?;
                    return Ok(());
                }
            }
        }

        if buffer.len() > limits::MAX_BUFFER_SIZE {
            stream
                .write_all(b"-ERR Protocol error: request too large\r\n")
                .await?;
            return Ok(());
        }
    }
}
