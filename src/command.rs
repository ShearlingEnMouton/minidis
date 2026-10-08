use crate::store::Store;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum Command {
    Ping {
        message: Option<Vec<u8>>,
    },
    Get {
        key: Vec<u8>,
    },
    Set {
        key: Vec<u8>,
        value: Vec<u8>,
        ttl: Option<Duration>,
    },
    Del {
        keys: Vec<Vec<u8>>,
    },
}

#[derive(Debug)]
pub enum CommandError {
    UnknownCommand,
    WrongArity,
    NotAnInteger,
    InvalidExpire,
    SyntaxError,
}

impl CommandError {
    pub fn to_resp(&self) -> &'static [u8] {
        match self {
            CommandError::UnknownCommand => b"-ERR unknown command\r\n",
            CommandError::WrongArity => b"-ERR wrong number of arguments\r\n",
            CommandError::NotAnInteger => b"-ERR value is not a valid integer\r\n",
            CommandError::InvalidExpire => b"-ERR invalid expiration time\r\n",
            CommandError::SyntaxError => b"-ERR syntax error\r\n",
        }
    }
}

impl Command {
    pub fn from_args(args: &[Vec<u8>]) -> Result<Command, CommandError> {
        let name = args.first().ok_or(CommandError::WrongArity)?;

        match name.to_ascii_uppercase().as_slice() {
            b"PING" => match args.len() {
                1 => Ok(Command::Ping { message: None }),
                2 => Ok(Command::Ping {
                    message: Some(args[1].clone()),
                }),
                _ => Err(CommandError::WrongArity),
            },
            b"GET" => match args.len() {
                2 => Ok(Command::Get {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity),
            },
            b"SET" => match args.len() {
                3 => Ok(Command::Set {
                    key: args[1].clone(),
                    value: args[2].clone(),
                    ttl: None,
                }),
                5 => {
                    let raw = args[4].clone();
                    let string_num =
                        std::str::from_utf8(&raw).map_err(|_| CommandError::NotAnInteger)?;
                    let parsed: u64 = string_num.parse().map_err(|_| CommandError::NotAnInteger)?;
                    let ttl = match args[3].to_ascii_uppercase().as_slice() {
                        b"EX" => Duration::from_secs(parsed),
                        b"PX" => Duration::from_millis(parsed),
                        _ => return Err(CommandError::SyntaxError),
                    };
                    if ttl.is_zero() || Instant::now().checked_add(ttl).is_none() {
                        return Err(CommandError::InvalidExpire);
                    }

                    Ok(Command::Set {
                        key: args[1].clone(),
                        value: args[2].clone(),
                        ttl: Some(ttl),
                    })
                }
                _ => Err(CommandError::WrongArity),
            },
            b"DEL" => match args.len() {
                nb if nb >= 2 => Ok(Command::Del {
                    keys: args[1..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity),
            },
            _ => Err(CommandError::UnknownCommand),
        }
    }

    pub fn execute(self, store: &Store) -> Vec<u8> {
        match self {
            Command::Ping { message: None } => b"+PONG\r\n".to_vec(),
            Command::Ping {
                message: Some(message),
            } => {
                let mut response = format!("${}\r\n", message.len()).into_bytes();
                response.extend_from_slice(&message);
                response.extend_from_slice(b"\r\n");
                response
            }
            Command::Get { key } => match store.get(&key) {
                None => b"$-1\r\n".to_vec(),
                Some(value) => {
                    let mut response = format!("${}\r\n", value.len()).into_bytes();
                    response.extend_from_slice(&value);
                    response.extend_from_slice(b"\r\n");
                    response
                }
            },
            Command::Set { key, value, ttl } => {
                store.set(key, value, ttl);
                b"+OK\r\n".to_vec()
            }
            Command::Del { keys } => {
                let removed = store.del(&keys);
                format!(":{removed}\r\n").into_bytes()
            }
        }
    }
}
