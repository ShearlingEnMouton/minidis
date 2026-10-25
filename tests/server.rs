use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    thread::sleep,
    time::{Duration, Instant},
};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

const SET_A: &[u8] = b"*3\r\n$3\r\nSET\r\n$1\r\na\r\n$1\r\n1\r\n";

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> TempDir {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("minidis-it-{}-{}", std::process::id(), id));
        fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    fn aof(&self) -> PathBuf {
        self.path.join("appendonly.aof")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct Server {
    child: Child,
    port: u16,
}

impl Server {
    fn start(aof: &PathBuf) -> Server {
        Server::start_with(aof, "always")
    }

    fn start_with(aof: &PathBuf, fsync: &str) -> Server {
        let mut server = Server::spawn(aof, fsync);
        server.wait_until_ready();
        server
    }

    fn spawn(aof: &PathBuf, fsync: &str) -> Server {
        Server::spawn_with_env(aof, fsync, &[])
    }

    fn spawn_with_env(aof: &PathBuf, fsync: &str, extra: &[(&str, &str)]) -> Server {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(env!("CARGO_BIN_EXE_minidis"))
            .env("BIND_ADDR", "127.0.0.1")
            .env("EXPOSE_PORT", port.to_string())
            .env("AOF_PATH", aof)
            .env("AOF_FSYNC", fsync)
            .env("LOG_LEVEL", "ERR")
            .envs(extra.iter().copied())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Server { child, port }
    }

    fn wait_until_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            sleep(Duration::from_millis(50));
        }
        panic!("server did not start listening");
    }

    fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
    }
}

fn send(stream: &mut TcpStream, parts: &[&str]) -> String {
    let mut buf = format!("*{}\r\n", parts.len());
    for part in parts {
        buf.push_str(&format!("${}\r\n{}\r\n", part.len(), part));
    }
    stream.write_all(buf.as_bytes()).unwrap();
    let mut reply = vec![0u8; 4096];
    let n = stream.read(&mut reply).unwrap();
    String::from_utf8(reply[..n].to_vec()).unwrap()
}

fn send_raw(stream: &mut TcpStream, bytes: &[u8], reply_len: usize) -> String {
    stream.write_all(bytes).unwrap();
    let mut reply = Vec::new();
    let mut chunk = [0u8; 4096];
    while reply.len() < reply_len {
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0, "connection closed after {} bytes", reply.len());
        reply.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8(reply).unwrap()
}

#[cfg(feature = "full")]
fn integer(reply: &str) -> i64 {
    reply
        .strip_prefix(':')
        .and_then(|r| r.strip_suffix("\r\n"))
        .unwrap_or_else(|| panic!("not an integer reply: {reply:?}"))
        .parse()
        .unwrap()
}

#[cfg(feature = "full")]
#[test]
fn data_survives_a_restart() {
    let dir = TempDir::new();
    let mut server = Server::start(&dir.aof());
    let mut c = server.connect();
    assert_eq!(send(&mut c, &["SET", "a", "1"]), "+OK\r\n");
    assert_eq!(send(&mut c, &["RPUSH", "l", "x", "y"]), ":2\r\n");
    assert_eq!(send(&mut c, &["HSET", "h", "f", "v"]), ":1\r\n");
    assert_eq!(send(&mut c, &["SET", "t", "v", "EX", "100"]), "+OK\r\n");
    drop(c);
    server.kill();

    let server = Server::start(&dir.aof());
    let mut c = server.connect();
    assert_eq!(send(&mut c, &["GET", "a"]), "$1\r\n1\r\n");
    assert_eq!(
        send(&mut c, &["LRANGE", "l", "0", "-1"]),
        "*2\r\n$1\r\nx\r\n$1\r\ny\r\n"
    );
    assert_eq!(send(&mut c, &["HGET", "h", "f"]), "$1\r\nv\r\n");
    assert_eq!(send(&mut c, &["DBSIZE"]), ":4\r\n");
    let ttl = integer(&send(&mut c, &["TTL", "t"]));
    assert!((90..=100).contains(&ttl), "ttl was {ttl}");
}

#[cfg(feature = "full")]
#[test]
fn expired_key_is_not_restored() {
    let dir = TempDir::new();
    let mut server = Server::start(&dir.aof());
    let mut c = server.connect();
    assert_eq!(send(&mut c, &["SET", "k", "v", "PX", "200"]), "+OK\r\n");
    assert_eq!(send(&mut c, &["SET", "keep", "v"]), "+OK\r\n");
    sleep(Duration::from_millis(500));
    drop(c);
    server.kill();

    let server = Server::start(&dir.aof());
    let mut c = server.connect();
    assert_eq!(send(&mut c, &["GET", "k"]), "$-1\r\n");
    assert_eq!(send(&mut c, &["DBSIZE"]), ":1\r\n");
}

#[cfg(feature = "full")]
#[test]
fn rewriteaof_shrinks_the_file_and_keeps_the_data() {
    let dir = TempDir::new();
    let mut server = Server::start(&dir.aof());
    let mut c = server.connect();
    for i in 0..50 {
        assert_eq!(send(&mut c, &["SET", "a", &i.to_string()]), "+OK\r\n");
    }
    let before = fs::metadata(dir.aof()).unwrap().len();

    assert_eq!(send(&mut c, &["REWRITEAOF"]), "+OK\r\n");

    let after = fs::metadata(dir.aof()).unwrap().len();
    assert!(after < before, "{after} >= {before}");
    assert_eq!(send(&mut c, &["SET", "b", "after"]), "+OK\r\n");
    drop(c);
    server.kill();

    let server = Server::start(&dir.aof());
    let mut c = server.connect();
    assert_eq!(send(&mut c, &["GET", "a"]), "$2\r\n49\r\n");
    assert_eq!(send(&mut c, &["GET", "b"]), "$5\r\nafter\r\n");
}

#[test]
fn an_incomplete_tail_is_truncated_on_start() {
    let dir = TempDir::new();
    let partial = b"*3\r\n$3\r\nSET\r\n$1\r\nb";
    fs::write(dir.aof(), [SET_A, &partial[..]].concat()).unwrap();

    let server = Server::start(&dir.aof());
    let mut c = server.connect();

    assert_eq!(send(&mut c, &["GET", "a"]), "$1\r\n1\r\n");
    #[cfg(feature = "full")]
    assert_eq!(send(&mut c, &["DBSIZE"]), ":1\r\n");
    assert_eq!(fs::metadata(dir.aof()).unwrap().len(), SET_A.len() as u64);
}

#[test]
fn a_corrupt_aof_stops_the_server_from_starting() {
    let dir = TempDir::new();
    fs::write(dir.aof(), [SET_A, b"?garbage\r\n", SET_A].concat()).unwrap();

    let mut server = Server::spawn(&dir.aof(), "always");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            assert!(!status.success());
            return;
        }
        assert!(Instant::now() < deadline, "server kept running");
        sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_invalid_fsync_policy_stops_the_server_from_starting() {
    let dir = TempDir::new();
    let mut server = Server::spawn(&dir.aof(), "bogus");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            assert!(!status.success());
            return;
        }
        assert!(Instant::now() < deadline, "server kept running");
        sleep(Duration::from_millis(50));
    }
}

#[cfg(feature = "full")]
#[test]
fn pipelined_commands_are_answered_in_order() {
    let dir = TempDir::new();
    let server = Server::start(&dir.aof());
    let mut c = server.connect();
    let batch = b"*3\r\n$3\r\nSET\r\n$1\r\na\r\n$1\r\n1\r\n\
*2\r\n$3\r\nGET\r\n$1\r\na\r\n\
*1\r\n$6\r\nDBSIZE\r\n";
    let expected = "+OK\r\n$1\r\n1\r\n:1\r\n";

    assert_eq!(send_raw(&mut c, batch, expected.len()), expected);
}

#[test]
fn a_command_split_across_two_writes_is_still_answered() {
    let dir = TempDir::new();
    let server = Server::start(&dir.aof());
    let mut c = server.connect();
    let command = b"*3\r\n$3\r\nSET\r\n$1\r\na\r\n$1\r\n1\r\n";
    let (first, second) = command.split_at(11);

    c.write_all(first).unwrap();
    c.flush().unwrap();
    sleep(Duration::from_millis(150));

    assert_eq!(send_raw(&mut c, second, 5), "+OK\r\n");
    assert_eq!(send(&mut c, &["GET", "a"]), "$1\r\n1\r\n");
}

#[test]
fn a_bad_command_does_not_close_the_connection() {
    let dir = TempDir::new();
    let server = Server::start(&dir.aof());
    let mut c = server.connect();

    assert!(send(&mut c, &["NOPE"]).starts_with("-ERR unknown command"));
    assert_eq!(
        send(&mut c, &["GET"]),
        "-ERR wrong number of arguments for 'get' command\r\n"
    );
    assert_eq!(send(&mut c, &["PING"]), "+PONG\r\n");
}

#[test]
fn a_protocol_error_closes_the_connection() {
    let dir = TempDir::new();
    let server = Server::start(&dir.aof());
    let mut c = server.connect();

    assert_eq!(send_raw(&mut c, b"?bad\r\n", 20), "-ERR Protocol error\r\n");
}

#[test]
fn writes_from_many_clients_are_all_restored() {
    let dir = TempDir::new();
    let mut server = Server::start(&dir.aof());
    let port = server.port;

    let workers: Vec<_> = (0..4)
        .map(|w| {
            std::thread::spawn(move || {
                let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
                c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                for i in 0..25 {
                    let key = format!("k-{w}-{i}");
                    assert_eq!(send(&mut c, &["SET", &key, "v"]), "+OK\r\n");
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    server.kill();

    let server = Server::start(&dir.aof());
    let mut c = server.connect();
    #[cfg(feature = "full")]
    assert_eq!(send(&mut c, &["DBSIZE"]), ":100\r\n");
    assert_eq!(send(&mut c, &["GET", "k-3-24"]), "$1\r\nv\r\n");
}

#[test]
fn data_survives_a_restart_under_every_fsync_policy() {
    for policy in ["always", "everysec", "no"] {
        let dir = TempDir::new();
        let mut server = Server::start_with(&dir.aof(), policy);
        let mut c = server.connect();
        assert_eq!(send(&mut c, &["SET", "a", "1"]), "+OK\r\n", "{policy}");
        #[cfg(feature = "full")]
        assert_eq!(send(&mut c, &["RPUSH", "l", "x"]), ":1\r\n", "{policy}");
        drop(c);
        server.kill();

        let server = Server::start_with(&dir.aof(), policy);
        let mut c = server.connect();
        assert_eq!(send(&mut c, &["GET", "a"]), "$1\r\n1\r\n", "{policy}");
        #[cfg(feature = "full")]
        assert_eq!(send(&mut c, &["LLEN", "l"]), ":1\r\n", "{policy}");
    }
}

#[test]
fn the_fsync_policy_name_is_case_insensitive() {
    let dir = TempDir::new();
    let server = Server::start_with(&dir.aof(), "EverySec");
    let mut c = server.connect();
    assert_eq!(send(&mut c, &["PING"]), "+PONG\r\n");
}

#[cfg(feature = "full")]
#[test]
fn invalid_purge_settings_stop_the_server_from_starting() {
    for (name, value) in [
        ("PURGE_LIMIT", "0"),
        ("PURGE_LIMIT", "abc"),
        ("PURGE_INTERVAL", "0"),
        ("PURGE_INTERVAL", "-5"),
    ] {
        let dir = TempDir::new();
        let mut server = Server::spawn_with_env(&dir.aof(), "always", &[(name, value)]);

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = server.child.try_wait().unwrap() {
                assert!(!status.success(), "{name}={value}");
                break;
            }
            assert!(Instant::now() < deadline, "{name}={value} kept running");
            sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(not(feature = "full"))]
#[test]
fn lite_data_survives_a_restart() {
    let dir = TempDir::new();
    let mut server = Server::start(&dir.aof());
    let mut c = server.connect();
    assert_eq!(send(&mut c, &["SET", "a", "1"]), "+OK\r\n");
    assert_eq!(send(&mut c, &["SET", "b", "2"]), "+OK\r\n");
    assert_eq!(send(&mut c, &["DEL", "a"]), ":1\r\n");
    drop(c);
    server.kill();

    let server = Server::start(&dir.aof());
    let mut c = server.connect();
    assert_eq!(send(&mut c, &["GET", "a"]), "$-1\r\n");
    assert_eq!(send(&mut c, &["GET", "b"]), "$1\r\n2\r\n");
}

#[cfg(not(feature = "full"))]
#[test]
fn lite_rejects_full_only_commands() {
    let dir = TempDir::new();
    let server = Server::start(&dir.aof());
    let mut c = server.connect();
    for parts in [
        &["TTL", "k"][..],
        &["EXPIRE", "k", "10"][..],
        &["LPUSH", "l", "x"][..],
        &["HSET", "h", "f", "v"][..],
        &["TYPE", "k"][..],
        &["DBSIZE"][..],
        &["REWRITEAOF"][..],
    ] {
        let reply = send(&mut c, parts);
        assert!(
            reply.starts_with("-ERR unknown command"),
            "{parts:?}: {reply:?}"
        );
    }
    assert_eq!(
        send(&mut c, &["SET", "k", "v", "EX", "10"]),
        "-ERR wrong number of arguments for 'set' command\r\n"
    );
    assert_eq!(send(&mut c, &["PING"]), "+PONG\r\n");
}

#[cfg(not(feature = "full"))]
#[test]
fn lite_refuses_an_aof_written_by_the_full_build() {
    let dir = TempDir::new();
    let rpush = b"*3\r\n$5\r\nRPUSH\r\n$1\r\nl\r\n$1\r\nx\r\n";
    fs::write(dir.aof(), [SET_A, &rpush[..]].concat()).unwrap();
    let before = fs::read(dir.aof()).unwrap();

    let mut server = Server::spawn(&dir.aof(), "always");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(Instant::now() < deadline, "server kept running");
        sleep(Duration::from_millis(50));
    }
    assert_eq!(fs::read(dir.aof()).unwrap(), before);
}
