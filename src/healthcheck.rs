use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use crate::env_or;

const TIMEOUT: Duration = Duration::from_secs(2);

pub fn healthz() -> std::io::Result<()> {
    let bind_addr = env_or("BIND_ADDR", "0.0.0.0");
    let host = match bind_addr.trim_start_matches('[').trim_end_matches(']') {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        other => other,
    };
    let port = env_or("EXPOSE_PORT", "3131");
    let port: u16 = port
        .parse()
        .map_err(|_| std::io::Error::other(format!("invalid EXPOSE_PORT: {port}")))?;

    let addr = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::other(format!("could not resolve {host}")))?;
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    stream.write_all(b"*1\r\n$4\r\nPING\r\n")?;
    let mut buf = [0u8; 7];
    stream.read_exact(&mut buf)?;
    if &buf == b"+PONG\r\n" {
        Ok(())
    } else {
        Err(std::io::Error::other("unexpected PING reply"))
    }
}
