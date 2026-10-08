use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::command::Command;
use crate::store::Store;
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

    let store = Store::new();

    loop {
        let (socket, peer_addr) = listener.accept().await?;
        log_info!("New client connected: {}", peer_addr);

        let store = store.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, &store).await {
                log_err!("Error handling client {}: {:?}", peer_addr, e);
            }
        });
    }
}

async fn handle_client(mut stream: TcpStream, store: &Store) -> tokio::io::Result<()> {
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);

    loop {
        let bytes_read = stream.read_buf(&mut buffer).await?;

        if bytes_read == 0 {
            log_info!("Client disconnected.");
            return Ok(());
        }

        loop {
            match resp::parse_command(&buffer) {
                Ok(Some((args, consumed))) => {
                    buffer.drain(..consumed);
                    if args.is_empty() {
                        continue;
                    }
                    match Command::from_args(&args) {
                        Ok(cmd) => {
                            let response = cmd.execute(store);
                            stream.write_all(&response).await?;
                        }
                        Err(e) => stream.write_all(&e.to_resp()).await?,
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
