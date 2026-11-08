use std::{
    net::SocketAddr,
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    signal::unix::{Signal, SignalKind, signal},
    time::{sleep, timeout},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[cfg(feature = "full")]
use tokio::{
    task,
    time::{MissedTickBehavior, interval},
};

use crate::aof::{Aof, FsyncPolicy};
use crate::command::{Command, CommandError, Protocol, Reply};
use crate::store::Store;
mod aof;
mod command;
mod healthcheck;
mod limits;
mod logger;
mod resp;
mod store;

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

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

    let max_clients = env_positive_u64("MAX_CLIENTS", 1000)?;
    let idle_timeout = env_duration("IDLE_TIMEOUT", 3600)?;
    let requirepass: Option<Arc<[u8]>> = std::env::var("REQUIREPASS")
        .ok()
        .filter(|p| !p.is_empty())
        .map(|p| Arc::from(p.into_bytes()));
    log_info!(
        "Authentication: {}",
        if requirepass.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );

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

                if tracker.len() as u64 >= max_clients {
                    log_warn!("max number of clients reached");
                    let _ = socket.try_write(b"-ERR max number of clients reached\r\n");
                    drop(socket);
                    continue;
                }

                let aof = aof.clone();
                let client_token = cancellation_token.clone();
                let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
                let requirepass = requirepass.clone();
                tracker.spawn(async move {
                    let ctx = ClientContext {
                        store: &store,
                        aof: &aof,
                        cancellation_token: &client_token,
                        idle_timeout,
                        client_id,
                        requirepass: requirepass.as_deref(),
                    };
                    if let Err(e) = handle_client(socket, peer_addr, ctx).await {
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

fn env_duration(name: &str, default: u64) -> Result<Option<Duration>, String> {
    match std::env::var(name) {
        Err(_) => Ok(Some(Duration::from_secs(default))),
        Ok(v) => match v.parse::<u64>() {
            Ok(n) => {
                if n == 0 {
                    Ok(None)
                } else {
                    Ok(Some(Duration::from_secs(n)))
                }
            }
            _ => Err(format!("{name} must be a non-negative integer")),
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

fn check_auth(
    requirepass: Option<&[u8]>,
    username: Option<&[u8]>,
    password: &[u8],
) -> Result<(), CommandError> {
    match requirepass {
        Some(p) => {
            if matches!(username, None | Some(b"default")) {
                if password.len() != p.len() {
                    return Err(CommandError::WrongPass);
                }

                if password
                    .iter()
                    .zip(p.iter())
                    .fold(0, |xor, (p_byte, rp_byte)| xor | p_byte ^ rp_byte)
                    == 0
                {
                    Ok(())
                } else {
                    Err(CommandError::WrongPass)
                }
            } else {
                Err(CommandError::WrongPass)
            }
        }
        None => match username {
            Some(u) => match u {
                b"default" => Ok(()),
                _ => Err(CommandError::WrongPass),
            },
            None => Err(CommandError::AuthNotConfigured),
        },
    }
}

fn dispatch(
    args: &[Vec<u8>],
    store: &Store,
    aof: &Aof,
    protocol: &mut Protocol,
    client_id: u64,
    authenticated: &mut bool,
    requirepass: Option<&[u8]>,
) -> Reply {
    let parsed = Command::from_args(args);

    if !*authenticated {
        let exempt = args
            .first()
            .is_some_and(|n| n.eq_ignore_ascii_case(b"AUTH") || n.eq_ignore_ascii_case(b"HELLO"))
            || matches!(
                parsed,
                Err(CommandError::UnknownCommand { .. } | CommandError::WrongArity(_))
            );
        if !exempt {
            return Reply::from(CommandError::NoAuth);
        }
    }

    let cmd = match parsed {
        Ok(cmd) => cmd,
        Err(e) => return Reply::from(e),
    };

    if let Command::Auth { username, password } = &cmd {
        match check_auth(requirepass, username.as_deref(), password) {
            Ok(_) => {
                *authenticated = true;
                return Reply::Simple("OK");
            }
            Err(e) => return Reply::from(e),
        }
    }

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
    if let Command::Hello { protover, auth } = cmd {
        if let Some((username, password)) = auth {
            match check_auth(requirepass, Some(&username), &password) {
                Ok(_) => *authenticated = true,
                Err(e) => return Reply::from(e),
            }
        }
        if !*authenticated {
            return Reply::from(CommandError::HelloNoAuth);
        }

        if let Some(p) = protover {
            *protocol = p;
        }
        let proto = match *protocol {
            Protocol::Resp2 => 2,
            Protocol::Resp3 => 3,
        };

        return Reply::Map(vec![
            (
                Reply::Bulk(b"server".to_vec()),
                Reply::Bulk(b"minidis".to_vec()),
            ),
            (
                Reply::Bulk(b"version".to_vec()),
                Reply::Bulk(env!("CARGO_PKG_VERSION").into()),
            ),
            (Reply::Bulk(b"proto".to_vec()), Reply::Integer(proto)),
            (
                Reply::Bulk(b"id".to_vec()),
                Reply::Integer(client_id as i64),
            ),
            (
                Reply::Bulk(b"mode".to_vec()),
                Reply::Bulk(b"standalone".to_vec()),
            ),
            (
                Reply::Bulk(b"role".to_vec()),
                Reply::Bulk(b"master".to_vec()),
            ),
            (Reply::Bulk(b"modules".to_vec()), Reply::Array(Vec::new())),
        ]);
    }
    if cmd.is_write() {
        aof.run_write(cmd, args, store)
    } else {
        cmd.execute(store)
    }
}

struct ClientContext<'a> {
    store: &'a Store,
    aof: &'a Aof,
    cancellation_token: &'a CancellationToken,
    idle_timeout: Option<Duration>,
    client_id: u64,
    requirepass: Option<&'a [u8]>,
}

async fn handle_client(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    ctx: ClientContext<'_>,
) -> tokio::io::Result<()> {
    let ClientContext {
        store,
        aof,
        cancellation_token,
        idle_timeout,
        client_id,
        requirepass,
    } = ctx;
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let mut protocol = Protocol::Resp2;
    let mut authenticated = requirepass.is_none();
    let mut seen_data = false;
    loop {
        let bytes_read = tokio::select! {
            res = stream.read_buf(&mut buffer) => res?,
            _ = cancellation_token.cancelled() => return Ok(()),
            _ = sleep(idle_timeout.unwrap_or_default()), if idle_timeout.is_some() => {
                log_debug!("Idle timeout: {}", peer_addr);
                return Ok(());
            },
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
                    dispatch(
                        &args,
                        store,
                        aof,
                        &mut protocol,
                        client_id,
                        &mut authenticated,
                        requirepass,
                    )
                    .encode(&mut out, &protocol);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_auth_with_a_password_configured() {
        let pass = Some(&b"s3cret"[..]);
        assert!(check_auth(pass, None, b"s3cret").is_ok());
        assert!(check_auth(pass, Some(b"default"), b"s3cret").is_ok());
        for (username, password) in [
            (None, &b"wrong"[..]),
            (None, b"s3cre"),
            (None, b"s3cret!"),
            (None, b""),
            (Some(&b"default"[..]), b"wrong"),
            (Some(b"other"), b"s3cret"),
        ] {
            assert!(
                matches!(
                    check_auth(pass, username, password),
                    Err(CommandError::WrongPass)
                ),
                "{username:?} {password:?}"
            );
        }
    }

    #[test]
    fn check_auth_without_a_password_configured() {
        assert!(matches!(
            check_auth(None, None, b"anything"),
            Err(CommandError::AuthNotConfigured)
        ));
        assert!(check_auth(None, Some(b"default"), b"anything").is_ok());
        assert!(matches!(
            check_auth(None, Some(b"other"), b"anything"),
            Err(CommandError::WrongPass)
        ));
    }
}
