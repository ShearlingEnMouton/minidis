use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    signal::unix::{Signal, SignalKind, signal},
    time::timeout,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[cfg(feature = "full")]
use tokio::{
    task,
    time::{MissedTickBehavior, interval},
};

use crate::aof::{Aof, FsyncPolicy};
use crate::command::{Command, Reply};
use crate::store::Store;
mod aof;
mod command;
mod healthcheck;
mod limits;
mod logger;
mod resp;
mod store;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        if let Err(e) = healthcheck::healthz() {
            eprintln!("healthcheck failed: {e}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    let bind_addr = env_or("BIND_ADDR", "0.0.0.0");
    let port = env_or("EXPOSE_PORT", "3131");
    #[cfg(feature = "full")]
    let purge_limit = env_positive_u64("PURGE_LIMIT", 1000)?;
    #[cfg(feature = "full")]
    let purge_int = env_positive_u64("PURGE_INTERVAL", 100)?;
    let aof_path = env_or("AOF_PATH", "/data/appendonly.aof");
    let policy = parse_fsync_policy(&env_or("AOF_FSYNC", "everysec"))?;
    let shutdown_timeout = Duration::from_secs(env_positive_u64("SHUTDOWN_TIMEOUT", 10)?);

    let sigint = signal(SignalKind::interrupt())?;
    let sigterm = signal(SignalKind::terminate())?;
    let cancellation_token = CancellationToken::new();
    tokio::spawn(shutdown_signal(sigint, sigterm, cancellation_token.clone()));

    let store = Store::new();
    let aof = Aof::open(&aof_path, policy)?;
    log_info!("AOF file: {}", aof_path);
    let replayed = Aof::replay(&aof_path, &store)?;
    log_info!("AOF replayed: {} commands", replayed);

    let addr = format!("{}:{}", bind_addr, port);
    let listener = TcpListener::bind(addr).await?;
    let bind_addr = listener.local_addr()?;
    log_info!("Listening on: {bind_addr}");

    aof.start_fsync_task();

    #[cfg(feature = "full")]
    let reaper_store = store.clone();
    #[cfg(feature = "full")]
    let reaper_token = cancellation_token.clone();
    #[cfg(feature = "full")]
    task::spawn(async move {
        let mut purge_interval = interval(Duration::from_millis(purge_int));
        purge_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            if reaper_token.is_cancelled() {
                break;
            }
            let purged = reaper_store.purge_expired(purge_limit);
            if purged > 0 {
                log_debug!("{purged} expired data have been purged.");
            }
            if purged == purge_limit {
                task::yield_now().await;
                continue;
            }

            tokio::select! {
                _ = purge_interval.tick() => {},
                _ = reaper_token.cancelled() => break,
            }
        }
    });

    let tracker = TaskTracker::new();
    let listener_token = cancellation_token.clone();
    loop {
        tokio::select! {
            _ = listener_token.cancelled() => {
                drop(listener);
                break
            },
            res = listener.accept() => {
                let (socket, peer_addr) = res?;
                let store = store.clone();
                let aof = aof.clone();
                let client_token = cancellation_token.clone();
                tracker.spawn(async move {
                    if let Err(e) = handle_client(socket, peer_addr, &store, &aof, &client_token).await {
                        log_err!("Error handling client {}: {:?}", peer_addr, e);
                    }
                });
            }
        }
    }

    tracker.close();

    if timeout(shutdown_timeout, tracker.wait()).await.is_err() {
        log_warn!(
            "Shutdown timed out after {:?} with {} client connection(s) still open, exiting anyway",
            shutdown_timeout,
            tracker.len()
        );
    }
    aof.fsync_before_shutdown()?;

    Ok(())
}

async fn shutdown_signal(mut sigint: Signal, mut sigterm: Signal, token: CancellationToken) {
    tokio::select! {
        _ = sigint.recv() => {
            log_info!("Received SIGINT (Ctrl+C), starting shutdown...");
        }
        _ = sigterm.recv() => {
            log_info!("Received SIGTERM (Termination), starting shutdown...");
        }
    }
    token.cancel();
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn env_positive_u64(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Err(_) => Ok(default),
        Ok(v) => match v.parse::<u64>() {
            Ok(n) if n > 0 => Ok(n),
            _ => Err(format!("{name} must be a positive integer")),
        },
    }
}

fn parse_fsync_policy(value: &str) -> Result<FsyncPolicy, String> {
    match value.to_ascii_lowercase().as_str() {
        "always" => Ok(FsyncPolicy::Always),
        "everysec" => Ok(FsyncPolicy::EverySec),
        "no" => Ok(FsyncPolicy::No),
        _ => Err("AOF_FSYNC must be always, everysec or no".to_string()),
    }
}

fn dispatch(args: &[Vec<u8>], store: &Store, aof: &Aof) -> Reply {
    let cmd = match Command::from_args(args) {
        Ok(cmd) => cmd,
        Err(e) => return Reply::from(e),
    };
    #[cfg(feature = "full")]
    if matches!(cmd, Command::Rewriteaof) {
        return match aof.rewrite(store) {
            Ok(()) => {
                log_info!("AOF rewritten");
                Reply::Simple("OK")
            }
            Err(e) => {
                log_err!("AOF rewrite failed: {}", e);
                Reply::Error(format!("ERR AOF rewrite failed: {e}"))
            }
        };
    }
    if cmd.is_write() {
        aof.run_write(cmd, args, store)
    } else {
        cmd.execute(store)
    }
}

async fn handle_client(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    store: &Store,
    aof: &Aof,
    cancellation_token: &CancellationToken,
) -> tokio::io::Result<()> {
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let mut seen_data = false;
    loop {
        let bytes_read = tokio::select! {
            res = stream.read_buf(&mut buffer) => res?,
            _ = cancellation_token.cancelled() => return Ok(()),
        };

        if bytes_read == 0 {
            if seen_data {
                log_debug!("Client disconnected: {}", peer_addr);
            } else {
                log_debug!("Connection closed without data: {}", peer_addr);
            }
            return Ok(());
        }

        if !seen_data {
            seen_data = true;
            log_debug!("New client connected: {}", peer_addr);
        }

        loop {
            match resp::parse_command(&buffer) {
                Ok(Some((args, consumed))) => {
                    buffer.drain(..consumed);
                    if args.is_empty() {
                        continue;
                    }
                    let mut out = Vec::new();
                    dispatch(&args, store, aof).encode(&mut out);
                    stream.write_all(&out).await?;
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
