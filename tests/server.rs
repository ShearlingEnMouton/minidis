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
        let mut server = Server::spawn(aof, "always");
        server.wait_until_ready();
        server
    }

    fn spawn(aof: &PathBuf, fsync: &str) -> Server {
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

fn integer(reply: &str) -> i64 {
    reply
        .strip_prefix(':')
        .and_then(|r| r.strip_suffix("\r\n"))
        .unwrap_or_else(|| panic!("not an integer reply: {reply:?}"))
        .parse()
        .unwrap()
}

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
