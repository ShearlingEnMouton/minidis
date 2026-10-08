use crate::store::{Store, TtlStatus};
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
    Ttl {
        key: Vec<u8>,
    },
    Pttl {
        key: Vec<u8>,
    },
    Expire {
        key: Vec<u8>,
        ttl: Duration,
    },
    Persist {
        key: Vec<u8>,
    },
}

#[derive(Debug)]
pub enum CommandError {
    UnknownCommand { name: String, args: String },
    WrongArity(&'static str),
    NotAnInteger,
    InvalidExpire(&'static str),
    SyntaxError,
}

impl CommandError {
    pub fn to_resp(&self) -> Vec<u8> {
        match self {
            CommandError::UnknownCommand { name, args } => {
                format!("-ERR unknown command '{name}', with args beginning with: {args}\r\n")
            }
            CommandError::WrongArity(cmd) => {
                format!("-ERR wrong number of arguments for '{cmd}' command\r\n")
            }
            CommandError::NotAnInteger => {
                "-ERR value is not an integer or out of range\r\n".to_string()
            }
            CommandError::InvalidExpire(cmd) => {
                format!("-ERR invalid expire time in '{cmd}' command\r\n")
            }
            CommandError::SyntaxError => "-ERR syntax error\r\n".to_string(),
        }
        .into_bytes()
    }

    fn unknown_command(args: &[Vec<u8>]) -> CommandError {
        let clean = |b: &[u8]| -> String {
            String::from_utf8_lossy(&b[..b.len().min(128)])
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect()
        };
        let mut rest = String::new();
        for arg in &args[1..] {
            if rest.len() >= 128 {
                break;
            }
            rest.push_str(&format!("'{}' ", clean(arg)));
        }
        CommandError::UnknownCommand {
            name: clean(&args[0]),
            args: rest,
        }
    }
}

impl Command {
    fn parse_ttl_arg(arg: &[u8]) -> Result<i64, CommandError> {
        let s = std::str::from_utf8(arg).map_err(|_| CommandError::NotAnInteger)?;
        s.parse().map_err(|_| CommandError::NotAnInteger)
    }
    fn expire_command(
        key: &[u8],
        arg: &[u8],
        unit: fn(u64) -> Duration,
        name: &'static str,
    ) -> Result<Command, CommandError> {
        let n = Self::parse_ttl_arg(arg)?;
        if n <= 0 {
            return Ok(Command::Del {
                keys: vec![key.to_vec()],
            });
        }
        let ttl = unit(n as u64);
        if Instant::now().checked_add(ttl).is_none() {
            return Err(CommandError::InvalidExpire(name));
        }
        Ok(Command::Expire {
            key: key.to_vec(),
            ttl,
        })
    }

    pub fn from_args(args: &[Vec<u8>]) -> Result<Command, CommandError> {
        let name = args.first().ok_or(CommandError::WrongArity(""))?;

        match name.to_ascii_uppercase().as_slice() {
            b"PING" => match args.len() {
                1 => Ok(Command::Ping { message: None }),
                2 => Ok(Command::Ping {
                    message: Some(args[1].clone()),
                }),
                _ => Err(CommandError::WrongArity("ping")),
            },
            b"GET" => match args.len() {
                2 => Ok(Command::Get {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("get")),
            },
            b"SET" => match args.len() {
                3 => Ok(Command::Set {
                    key: args[1].clone(),
                    value: args[2].clone(),
                    ttl: None,
                }),
                5 => {
                    let parsed: i64 = Self::parse_ttl_arg(&args[4])?;
                    if parsed <= 0 {
                        return Err(CommandError::InvalidExpire("set"));
                    }
                    let ttl = match args[3].to_ascii_uppercase().as_slice() {
                        b"EX" => Duration::from_secs(parsed as u64),
                        b"PX" => Duration::from_millis(parsed as u64),
                        _ => return Err(CommandError::SyntaxError),
                    };
                    if Instant::now().checked_add(ttl).is_none() {
                        return Err(CommandError::InvalidExpire("set"));
                    }

                    Ok(Command::Set {
                        key: args[1].clone(),
                        value: args[2].clone(),
                        ttl: Some(ttl),
                    })
                }
                _ => Err(CommandError::WrongArity("set")),
            },
            b"DEL" => match args.len() {
                nb if nb >= 2 => Ok(Command::Del {
                    keys: args[1..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity("del")),
            },
            b"TTL" => match args.len() {
                2 => Ok(Command::Ttl {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("ttl")),
            },
            b"PTTL" => match args.len() {
                2 => Ok(Command::Pttl {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("pttl")),
            },
            b"EXPIRE" => match args.len() {
                3 => Self::expire_command(&args[1], &args[2], Duration::from_secs, "expire"),
                _ => Err(CommandError::WrongArity("expire")),
            },
            b"PEXPIRE" => match args.len() {
                3 => Self::expire_command(&args[1], &args[2], Duration::from_millis, "pexpire"),
                _ => Err(CommandError::WrongArity("pexpire")),
            },
            b"PERSIST" => match args.len() {
                2 => Ok(Command::Persist {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("persist")),
            },
            _ => Err(CommandError::unknown_command(args)),
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
            Command::Ttl { key } => match store.ttl(&key) {
                TtlStatus::NoKey => b":-2\r\n".to_vec(),
                TtlStatus::NoExpiry => b":-1\r\n".to_vec(),
                TtlStatus::Remaining(d) => {
                    format!(":{}\r\n", (d.as_millis() + 500) / 1000).into_bytes()
                }
            },
            Command::Pttl { key } => match store.ttl(&key) {
                TtlStatus::NoKey => b":-2\r\n".to_vec(),
                TtlStatus::NoExpiry => b":-1\r\n".to_vec(),
                TtlStatus::Remaining(d) => format!(":{}\r\n", d.as_millis()).into_bytes(),
            },
            Command::Expire { key, ttl } => {
                let result = store.expire(&key, ttl);
                format!(":{result}\r\n").into_bytes()
            }
            Command::Persist { key } => {
                let result = store.persist(&key);
                format!(":{result}\r\n").into_bytes()
            }
        }
    }
}
