mod error;
mod reply;
mod types;

use crate::store::Store;
#[cfg(feature = "full")]
use crate::store::TtlStatus;
use error::CommandError;
pub use reply::{Protocol, Reply};
#[cfg(feature = "full")]
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
pub use types::Command;

impl Command {
    fn parse_int(arg: &[u8]) -> Result<i64, CommandError> {
        let s = std::str::from_utf8(arg).map_err(|_| CommandError::NotAnInteger)?;
        s.parse().map_err(|_| CommandError::NotAnInteger)
    }
    #[cfg(feature = "full")]
    fn expire_command(
        key: &[u8],
        arg: &[u8],
        unit: fn(u64) -> Duration,
        name: &'static str,
    ) -> Result<Command, CommandError> {
        let n = Self::parse_int(arg)?;
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
    #[cfg(feature = "full")]
    fn calculate_remaining(expire_at: u64, unit: fn(u64) -> Duration) -> Duration {
        (UNIX_EPOCH + unit(expire_at))
            .duration_since(SystemTime::now())
            .unwrap_or_default()
    }
    #[cfg(feature = "full")]
    fn expire_at_command(
        key: &[u8],
        arg: &[u8],
        unit: fn(u64) -> Duration,
        name: &'static str,
    ) -> Result<Command, CommandError> {
        let n = Self::parse_int(arg)?;
        if n <= 0 {
            return Ok(Command::Del {
                keys: vec![key.to_vec()],
            });
        }
        let ttl = Self::calculate_remaining(n as u64, unit);
        if ttl.is_zero() {
            return Ok(Command::Del {
                keys: vec![key.to_vec()],
            });
        }
        if Instant::now().checked_add(ttl).is_none() {
            return Err(CommandError::InvalidExpire(name));
        }
        Ok(Command::Expire {
            key: key.to_vec(),
            ttl,
        })
    }

    #[cfg(feature = "full")]
    fn scan_command(args: &[Vec<u8>]) -> Result<Command, CommandError> {
        if args.len() < 2 {
            return Err(CommandError::WrongArity("scan"));
        }
        let cursor = std::str::from_utf8(&args[1])
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or(CommandError::InvalidCursor)?;
        let mut pattern = None;
        let mut count = 10;
        let mut options = args[2..].iter();
        while let Some(option) = options.next() {
            let value = options.next().ok_or(CommandError::SyntaxError)?;
            match option.to_ascii_uppercase().as_slice() {
                b"MATCH" => pattern = Some(value.clone()),
                b"COUNT" => {
                    count = match Self::parse_int(value)? {
                        n if n >= 1 => n as usize,
                        _ => return Err(CommandError::SyntaxError),
                    }
                }
                _ => return Err(CommandError::SyntaxError),
            }
        }
        Ok(Command::Scan {
            cursor,
            pattern,
            count,
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
            b"HELLO" => match args.len() {
                1 => Ok(Command::Hello { protover: None }),
                2 => {
                    let protocol = match Self::parse_int(&args[1])? {
                        2 => Protocol::Resp2,
                        3 => Protocol::Resp3,
                        _ => return Err(CommandError::NoProto),
                    };

                    Ok(Command::Hello {
                        protover: Some(protocol),
                    })
                }
                _ => Err(CommandError::WrongArity("hello")),
            },
            b"INFO" => match args.len() {
                1 => Ok(Command::Info { section: None }),
                2 => Ok(Command::Info {
                    section: Some(args[1].clone()),
                }),
                _ => Err(CommandError::WrongArity("info")),
            },
            b"COMMAND" => match args.len() {
                1 => Ok(Command::Commands),
                _ if args[1].eq_ignore_ascii_case(b"DOCS") => Ok(Command::CommandsDocs),
                _ => Err(CommandError::unknown_command(args)),
            },
            b"CLIENT" => match args.len() {
                1 => Err(CommandError::WrongArity("client")),
                _ if args[1].eq_ignore_ascii_case(b"SETINFO") => match args.len() {
                    4 => Ok(Command::ClientSetinfo),
                    _ => Err(CommandError::WrongArity("client|setinfo")),
                },
                _ => Err(CommandError::unknown_command(args)),
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
                    #[cfg(feature = "full")]
                    ttl: None,
                }),
                #[cfg(feature = "full")]
                5 => {
                    let parsed: i64 = Self::parse_int(&args[4])?;
                    if parsed <= 0 {
                        return Err(CommandError::InvalidExpire("set"));
                    }
                    let ttl = match args[3].to_ascii_uppercase().as_slice() {
                        b"EX" => Duration::from_secs(parsed as u64),
                        b"PX" => Duration::from_millis(parsed as u64),
                        b"EXAT" => Self::calculate_remaining(parsed as u64, Duration::from_secs),
                        b"PXAT" => Self::calculate_remaining(parsed as u64, Duration::from_millis),
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
                2.. => Ok(Command::Del {
                    keys: args[1..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity("del")),
            },
            #[cfg(feature = "full")]
            b"EXISTS" => match args.len() {
                2.. => Ok(Command::Exists {
                    keys: args[1..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity("exists")),
            },
            #[cfg(feature = "full")]
            b"SETNX" => match args.len() {
                3 => Ok(Command::Setnx {
                    key: args[1].clone(),
                    value: args[2].clone(),
                }),
                _ => Err(CommandError::WrongArity("setnx")),
            },
            #[cfg(feature = "full")]
            b"MSET" => match args.len() {
                n if n >= 3 && n % 2 == 1 => Ok(Command::Mset {
                    pairs: args[1..]
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|p| (p[0].clone(), p[1].clone()))
                        .collect(),
                }),
                _ => Err(CommandError::WrongArity("mset")),
            },
            #[cfg(feature = "full")]
            b"MGET" => match args.len() {
                2.. => Ok(Command::Mget {
                    keys: args[1..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity("mget")),
            },
            #[cfg(feature = "full")]
            b"APPEND" => match args.len() {
                3 => Ok(Command::Append {
                    key: args[1].clone(),
                    value: args[2].clone(),
                }),
                _ => Err(CommandError::WrongArity("append")),
            },
            #[cfg(feature = "full")]
            b"INCR" => match args.len() {
                2 => Ok(Command::Incr {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("incr")),
            },
            #[cfg(feature = "full")]
            b"DECR" => match args.len() {
                2 => Ok(Command::Decr {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("decr")),
            },
            #[cfg(feature = "full")]
            b"KEYS" => match args.len() {
                2 => Ok(Command::Keys {
                    pattern: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("keys")),
            },
            #[cfg(feature = "full")]
            b"SCAN" => Self::scan_command(args),
            #[cfg(feature = "full")]
            b"TTL" => match args.len() {
                2 => Ok(Command::Ttl {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("ttl")),
            },
            #[cfg(feature = "full")]
            b"PTTL" => match args.len() {
                2 => Ok(Command::Pttl {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("pttl")),
            },
            #[cfg(feature = "full")]
            b"EXPIRE" => match args.len() {
                3 => Self::expire_command(&args[1], &args[2], Duration::from_secs, "expire"),
                _ => Err(CommandError::WrongArity("expire")),
            },
            #[cfg(feature = "full")]
            b"PEXPIRE" => match args.len() {
                3 => Self::expire_command(&args[1], &args[2], Duration::from_millis, "pexpire"),
                _ => Err(CommandError::WrongArity("pexpire")),
            },
            #[cfg(feature = "full")]
            b"PEXPIREAT" => match args.len() {
                3 => {
                    Self::expire_at_command(&args[1], &args[2], Duration::from_millis, "pexpireat")
                }
                _ => Err(CommandError::WrongArity("pexpireat")),
            },
            #[cfg(feature = "full")]
            b"PERSIST" => match args.len() {
                2 => Ok(Command::Persist {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("persist")),
            },
            #[cfg(feature = "full")]
            b"REWRITEAOF" => match args.len() {
                1 => Ok(Command::Rewriteaof),
                _ => Err(CommandError::WrongArity("rewriteaof")),
            },
            #[cfg(feature = "full")]
            b"DBSIZE" => match args.len() {
                1 => Ok(Command::Dbsize {}),
                _ => Err(CommandError::WrongArity("dbsize")),
            },
            #[cfg(feature = "full")]
            b"LPUSH" => match args.len() {
                3.. => Ok(Command::Lpush {
                    key: args[1].clone(),
                    values: args[2..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity("lpush")),
            },
            #[cfg(feature = "full")]
            b"RPUSH" => match args.len() {
                3.. => Ok(Command::Rpush {
                    key: args[1].clone(),
                    values: args[2..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity("rpush")),
            },
            #[cfg(feature = "full")]
            b"LPOP" => match args.len() {
                2 => Ok(Command::Lpop {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("lpop")),
            },
            #[cfg(feature = "full")]
            b"RPOP" => match args.len() {
                2 => Ok(Command::Rpop {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("rpop")),
            },
            #[cfg(feature = "full")]
            b"LLEN" => match args.len() {
                2 => Ok(Command::Llen {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("llen")),
            },
            #[cfg(feature = "full")]
            b"LRANGE" => match args.len() {
                4 => Ok(Command::Lrange {
                    key: args[1].clone(),
                    start: Self::parse_int(&args[2])?,
                    stop: Self::parse_int(&args[3])?,
                }),
                _ => Err(CommandError::WrongArity("lrange")),
            },
            #[cfg(feature = "full")]
            b"HSET" => match args.len() {
                n if n >= 4 && n % 2 == 0 => Ok(Command::Hset {
                    key: args[1].clone(),
                    pairs: args[2..]
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|p| (p[0].clone(), p[1].clone()))
                        .collect(),
                }),
                _ => Err(CommandError::WrongArity("hset")),
            },
            #[cfg(feature = "full")]
            b"HGET" => match args.len() {
                3 => Ok(Command::Hget {
                    key: args[1].clone(),
                    field: args[2].clone(),
                }),
                _ => Err(CommandError::WrongArity("hget")),
            },
            #[cfg(feature = "full")]
            b"HDEL" => match args.len() {
                3.. => Ok(Command::Hdel {
                    key: args[1].clone(),
                    fields: args[2..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity("hdel")),
            },
            #[cfg(feature = "full")]
            b"HLEN" => match args.len() {
                2 => Ok(Command::Hlen {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("hlen")),
            },
            #[cfg(feature = "full")]
            b"HEXISTS" => match args.len() {
                3 => Ok(Command::Hexists {
                    key: args[1].clone(),
                    field: args[2].clone(),
                }),
                _ => Err(CommandError::WrongArity("hexists")),
            },
            #[cfg(feature = "full")]
            b"HGETALL" => match args.len() {
                2 => Ok(Command::Hgetall {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("hgetall")),
            },
            #[cfg(feature = "full")]
            b"TYPE" => match args.len() {
                2 => Ok(Command::Type {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("type")),
            },
            _ => Err(CommandError::unknown_command(args)),
        }
    }

    fn info_text(section: Option<&[u8]>) -> Vec<u8> {
        let server = format!(
            "# Server\r\nminidis_version:{}\r\nredis_mode:standalone\r\n",
            env!("CARGO_PKG_VERSION")
        );
        let persistence = "# Persistence\r\nloading:0\r\n";
        let text = match section {
            None => format!("{server}\r\n{persistence}"),
            Some(name) if name.eq_ignore_ascii_case(b"server") => server,
            Some(name) if name.eq_ignore_ascii_case(b"persistence") => persistence.to_owned(),
            Some(_) => String::new(),
        };
        text.into_bytes()
    }

    pub fn execute(self, store: &Store) -> Reply {
        match self {
            Command::Ping { message: None } => Reply::Simple("PONG"),
            Command::Ping {
                message: Some(message),
            } => Reply::Bulk(message),
            Command::Hello { .. } => {
                Reply::Error("NOPROTO unsupported protocol version".to_owned())
            }
            Command::Info { section } => Reply::Bulk(Self::info_text(section.as_deref())),
            Command::Commands => Reply::Array(Vec::new()),
            Command::CommandsDocs => Reply::Map(Vec::new()),
            Command::ClientSetinfo => Reply::Simple("OK"),
            Command::Get { key } => match store.get(&key) {
                Ok(None) => Reply::Nil,
                Ok(Some(value)) => Reply::Bulk(value),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Set {
                key,
                value,
                #[cfg(feature = "full")]
                ttl,
            } => {
                store.set(
                    key,
                    value,
                    #[cfg(feature = "full")]
                    ttl,
                );
                Reply::Simple("OK")
            }
            Command::Del { keys } => {
                let removed = store.del(&keys);
                Reply::Integer(removed as i64)
            }
            #[cfg(feature = "full")]
            Command::Exists { keys } => Reply::Integer(store.exists(&keys) as i64),
            #[cfg(feature = "full")]
            Command::Setnx { key, value } => Reply::Integer(store.setnx(key, value) as i64),
            #[cfg(feature = "full")]
            Command::Mset { pairs } => {
                store.mset(pairs);
                Reply::Simple("OK")
            }
            #[cfg(feature = "full")]
            Command::Mget { keys } => Reply::Array(
                store
                    .mget(&keys)
                    .into_iter()
                    .map(|v| v.map_or(Reply::Nil, Reply::Bulk))
                    .collect(),
            ),
            #[cfg(feature = "full")]
            Command::Append { key, value } => match store.append(key, value) {
                Ok(len) => Reply::Integer(len as i64),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Incr { key } => match store.incr_by(key, 1) {
                Ok(n) => Reply::Integer(n),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Decr { key } => match store.incr_by(key, -1) {
                Ok(n) => Reply::Integer(n),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Keys { pattern } => {
                Reply::Array(store.keys(&pattern).into_iter().map(Reply::Bulk).collect())
            }
            #[cfg(feature = "full")]
            Command::Scan {
                cursor,
                pattern,
                count,
            } => {
                let (next, keys) = store.scan(cursor, pattern.as_deref(), count);
                Reply::Array(vec![
                    Reply::Bulk(next.to_string().into_bytes()),
                    Reply::Array(keys.into_iter().map(Reply::Bulk).collect()),
                ])
            }
            #[cfg(feature = "full")]
            Command::Ttl { key } => match store.ttl(&key) {
                TtlStatus::NoKey => Reply::Integer(-2),
                TtlStatus::NoExpiry => Reply::Integer(-1),
                TtlStatus::Remaining(d) => Reply::Integer(((d.as_millis() + 500) / 1000) as i64),
            },
            #[cfg(feature = "full")]
            Command::Pttl { key } => match store.ttl(&key) {
                TtlStatus::NoKey => Reply::Integer(-2),
                TtlStatus::NoExpiry => Reply::Integer(-1),
                TtlStatus::Remaining(d) => Reply::Integer(d.as_millis() as i64),
            },
            #[cfg(feature = "full")]
            Command::Expire { key, ttl } => {
                let result = store.expire(&key, ttl);
                Reply::Integer(result.into())
            }
            #[cfg(feature = "full")]
            Command::Persist { key } => {
                let result = store.persist(&key);
                Reply::Integer(result.into())
            }
            #[cfg(feature = "full")]
            Command::Dbsize {} => Reply::Integer(store.dbsize() as i64),
            #[cfg(feature = "full")]
            Command::Rewriteaof => Reply::Error("ERR REWRITEAOF is handled by the AOF".to_string()),
            #[cfg(feature = "full")]
            Command::Lpush { key, values } => match store.lpush(key, values) {
                Ok(len) => Reply::Integer(len as i64),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Rpush { key, values } => match store.rpush(key, values) {
                Ok(len) => Reply::Integer(len as i64),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Lpop { key } => match store.lpop(&key) {
                Ok(None) => Reply::Nil,
                Ok(Some(v)) => Reply::Bulk(v),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Rpop { key } => match store.rpop(&key) {
                Ok(None) => Reply::Nil,
                Ok(Some(v)) => Reply::Bulk(v),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Llen { key } => match store.llen(&key) {
                Ok(len) => Reply::Integer(len as i64),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Lrange { key, start, stop } => match store.lrange(&key, start, stop) {
                Ok(items) => Reply::Array(items.into_iter().map(Reply::Bulk).collect()),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Hset { key, pairs } => match store.hset(key, pairs) {
                Ok(added) => Reply::Integer(added as i64),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Hget { key, field } => match store.hget(&key, &field) {
                Ok(None) => Reply::Nil,
                Ok(Some(v)) => Reply::Bulk(v),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Hdel { key, fields } => match store.hdel(&key, &fields) {
                Ok(removed) => Reply::Integer(removed as i64),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Hlen { key } => match store.hlen(&key) {
                Ok(len) => Reply::Integer(len as i64),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Hexists { key, field } => match store.hexists(&key, &field) {
                Ok(exists) => Reply::Integer(exists.into()),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Hgetall { key } => match store.hgetall(&key) {
                Ok(pairs) => Reply::Map(
                    pairs
                        .into_iter()
                        .map(|(f, v)| (Reply::Bulk(f), Reply::Bulk(v)))
                        .collect(),
                ),
                Err(e) => CommandError::from(e).into(),
            },
            #[cfg(feature = "full")]
            Command::Type { key } => Reply::Simple(store.key_type(&key)),
        }
    }

    pub fn is_write(&self) -> bool {
        match self {
            Command::Set { .. } | Command::Del { .. } => true,
            #[cfg(feature = "full")]
            Command::Expire { .. }
            | Command::Persist { .. }
            | Command::Lpush { .. }
            | Command::Rpush { .. }
            | Command::Lpop { .. }
            | Command::Rpop { .. }
            | Command::Hset { .. }
            | Command::Hdel { .. }
            | Command::Setnx { .. }
            | Command::Mset { .. }
            | Command::Append { .. }
            | Command::Incr { .. }
            | Command::Decr { .. } => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(parts: &[&str]) -> Result<Command, CommandError> {
        let args: Vec<Vec<u8>> = parts.iter().map(|p| p.as_bytes().to_vec()).collect();
        Command::from_args(&args)
    }

    #[cfg(feature = "full")]
    fn unix_now() -> Duration {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
    }

    #[cfg(feature = "full")]
    fn set_ttl(parts: &[&str]) -> Option<Duration> {
        match parse(parts).unwrap() {
            Command::Set { ttl, .. } => ttl,
            other => panic!("expected SET, got {other:?}"),
        }
    }

    #[cfg(feature = "full")]
    fn expire_ttl(parts: &[&str]) -> Duration {
        match parse(parts).unwrap() {
            Command::Expire { ttl, .. } => ttl,
            other => panic!("expected an expire command, got {other:?}"),
        }
    }

    #[cfg(feature = "full")]
    fn is_del(parts: &[&str], key: &str) -> bool {
        match parse(parts).unwrap() {
            Command::Del { keys } => keys == vec![key.as_bytes().to_vec()],
            _ => false,
        }
    }

    #[cfg(feature = "full")]
    #[test]
    fn set_without_options_has_no_ttl() {
        assert_eq!(set_ttl(&["SET", "k", "v"]), None);
    }

    #[cfg(feature = "full")]
    #[test]
    fn set_ex_and_px_use_the_given_duration() {
        assert_eq!(
            set_ttl(&["SET", "k", "v", "EX", "10"]),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            set_ttl(&["SET", "k", "v", "PX", "1500"]),
            Some(Duration::from_millis(1500))
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn command_and_option_names_are_case_insensitive() {
        assert_eq!(
            set_ttl(&["set", "k", "v", "ex", "10"]),
            Some(Duration::from_secs(10))
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn set_exat_and_pxat_become_a_remaining_duration() {
        let at_secs = (unix_now().as_secs() + 100).to_string();
        let ttl = set_ttl(&["SET", "k", "v", "EXAT", at_secs.as_str()]).unwrap();
        assert!(ttl > Duration::from_secs(97) && ttl <= Duration::from_secs(100));

        let at_ms = (unix_now().as_millis() + 100_000).to_string();
        let ttl = set_ttl(&["SET", "k", "v", "PXAT", at_ms.as_str()]).unwrap();
        assert!(ttl > Duration::from_secs(97) && ttl <= Duration::from_secs(100));
    }

    #[cfg(feature = "full")]
    #[test]
    fn set_rejects_invalid_options() {
        assert!(matches!(
            parse(&["SET", "k", "v", "EX", "0"]),
            Err(CommandError::InvalidExpire("set"))
        ));
        assert!(matches!(
            parse(&["SET", "k", "v", "PX", "-5"]),
            Err(CommandError::InvalidExpire("set"))
        ));
        assert!(matches!(
            parse(&["SET", "k", "v", "EX", "abc"]),
            Err(CommandError::NotAnInteger)
        ));
        assert!(matches!(
            parse(&["SET", "k", "v", "FOO", "10"]),
            Err(CommandError::SyntaxError)
        ));
        assert!(matches!(
            parse(&["SET", "k", "v", "EX"]),
            Err(CommandError::WrongArity("set"))
        ));
        assert!(matches!(
            parse(&["SET", "k"]),
            Err(CommandError::WrongArity("set"))
        ));
    }

    #[cfg(feature = "full")]
    #[test]
    fn expire_and_pexpire_use_the_given_duration() {
        assert_eq!(expire_ttl(&["EXPIRE", "k", "10"]), Duration::from_secs(10));
        assert_eq!(
            expire_ttl(&["PEXPIRE", "k", "1500"]),
            Duration::from_millis(1500)
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn non_positive_expire_becomes_del() {
        assert!(is_del(&["EXPIRE", "k", "0"], "k"));
        assert!(is_del(&["EXPIRE", "k", "-5"], "k"));
        assert!(is_del(&["PEXPIRE", "k", "0"], "k"));
    }

    #[cfg(feature = "full")]
    #[test]
    fn expire_rejects_a_duration_that_overflows() {
        assert!(matches!(
            parse(&["EXPIRE", "k", "9223372036854775807"]),
            Err(CommandError::InvalidExpire("expire"))
        ));
    }

    #[cfg(feature = "full")]
    #[test]
    fn pexpireat_in_the_future_becomes_a_remaining_duration() {
        let at_ms = (unix_now().as_millis() + 100_000).to_string();
        let ttl = expire_ttl(&["PEXPIREAT", "k", at_ms.as_str()]);
        assert!(ttl > Duration::from_secs(97) && ttl <= Duration::from_secs(100));
    }

    #[cfg(feature = "full")]
    #[test]
    fn pexpireat_in_the_past_becomes_del() {
        assert!(is_del(&["PEXPIREAT", "k", "1000"], "k"));
        assert!(is_del(&["PEXPIREAT", "k", "0"], "k"));
        assert!(is_del(&["PEXPIREAT", "k", "-1"], "k"));
    }

    #[cfg(feature = "full")]
    #[test]
    fn rewriteaof_takes_no_arguments() {
        assert!(matches!(parse(&["REWRITEAOF"]), Ok(Command::Rewriteaof)));
        assert!(matches!(
            parse(&["REWRITEAOF", "now"]),
            Err(CommandError::WrongArity("rewriteaof"))
        ));
    }

    #[test]
    fn hello_takes_an_optional_protocol_version() {
        assert!(matches!(
            parse(&["HELLO"]),
            Ok(Command::Hello { protover: None })
        ));
        assert!(matches!(
            parse(&["HELLO", "2"]),
            Ok(Command::Hello {
                protover: Some(Protocol::Resp2)
            })
        ));
        assert!(matches!(
            parse(&["hello", "3"]),
            Ok(Command::Hello {
                protover: Some(Protocol::Resp3)
            })
        ));
        for version in ["0", "1", "4", "-3"] {
            assert!(
                matches!(parse(&["HELLO", version]), Err(CommandError::NoProto)),
                "{version}"
            );
        }
        assert!(matches!(
            parse(&["HELLO", "3", "extra"]),
            Err(CommandError::WrongArity("hello"))
        ));
    }

    #[test]
    fn info_takes_an_optional_section() {
        assert!(matches!(
            parse(&["INFO"]),
            Ok(Command::Info { section: None })
        ));
        assert!(matches!(
            parse(&["INFO", "server"]),
            Ok(Command::Info { section: Some(s) }) if s == b"server"
        ));
        assert!(matches!(
            parse(&["INFO", "a", "b"]),
            Err(CommandError::WrongArity("info"))
        ));
    }

    #[test]
    fn command_takes_no_arguments_or_docs() {
        assert!(matches!(parse(&["COMMAND"]), Ok(Command::Commands)));
        for parts in [
            &["COMMAND", "DOCS"][..],
            &["command", "docs"][..],
            &["COMMAND", "DOCS", "get", "set"][..],
        ] {
            assert!(
                matches!(parse(parts), Ok(Command::CommandsDocs)),
                "{parts:?}"
            );
        }
        assert!(matches!(
            parse(&["COMMAND", "COUNT"]),
            Err(CommandError::UnknownCommand { .. })
        ));
    }

    #[test]
    fn client_accepts_only_setinfo() {
        for parts in [
            &["CLIENT", "SETINFO", "LIB-NAME", "redis-py"][..],
            &["client", "setinfo", "LIB-VER", "1.0"][..],
        ] {
            assert!(
                matches!(parse(parts), Ok(Command::ClientSetinfo)),
                "{parts:?}"
            );
        }
        assert!(matches!(
            parse(&["CLIENT"]),
            Err(CommandError::WrongArity("client"))
        ));
        assert!(matches!(
            parse(&["CLIENT", "SETINFO", "LIB-NAME"]),
            Err(CommandError::WrongArity("client|setinfo"))
        ));
        assert!(matches!(
            parse(&["CLIENT", "ID"]),
            Err(CommandError::UnknownCommand { .. })
        ));
    }

    #[test]
    fn handshake_commands_are_not_writes() {
        for parts in [
            &["HELLO"][..],
            &["INFO"][..],
            &["COMMAND"][..],
            &["COMMAND", "DOCS"][..],
            &["CLIENT", "SETINFO", "LIB-NAME", "x"][..],
        ] {
            assert!(!parse(parts).unwrap().is_write(), "{parts:?}");
        }
    }

    #[cfg(feature = "full")]
    #[test]
    fn wrong_argument_counts_are_rejected() {
        for parts in [
            &["GET"][..],
            &["GET", "a", "b"][..],
            &["DEL"][..],
            &["TTL"][..],
            &["EXPIRE", "k"][..],
            &["PEXPIREAT", "k"][..],
            &["PERSIST"][..],
            &["LPUSH", "k"][..],
            &["DBSIZE", "x"][..],
        ] {
            assert!(
                matches!(parse(parts), Err(CommandError::WrongArity(_))),
                "{parts:?}"
            );
        }
    }

    #[test]
    fn unknown_and_empty_commands_are_rejected() {
        assert!(matches!(
            parse(&["NOPE", "a"]),
            Err(CommandError::UnknownCommand { .. })
        ));
        assert!(matches!(parse(&[]), Err(CommandError::WrongArity(_))));
    }

    #[cfg(feature = "full")]
    #[test]
    fn string_commands_check_their_argument_counts() {
        assert!(matches!(
            parse(&["EXISTS", "a", "a"]),
            Ok(Command::Exists { keys }) if keys.len() == 2
        ));
        assert!(matches!(
            parse(&["MGET", "a", "b"]),
            Ok(Command::Mget { keys }) if keys.len() == 2
        ));
        assert!(matches!(
            parse(&["SETNX", "k", "v"]),
            Ok(Command::Setnx { .. })
        ));
        assert!(matches!(
            parse(&["APPEND", "k", "v"]),
            Ok(Command::Append { .. })
        ));
        assert!(matches!(parse(&["INCR", "k"]), Ok(Command::Incr { .. })));
        assert!(matches!(parse(&["decr", "k"]), Ok(Command::Decr { .. })));
        assert!(matches!(
            parse(&["MSET", "a", "1", "b", "2"]),
            Ok(Command::Mset { pairs })
                if pairs == vec![(b"a".to_vec(), b"1".to_vec()), (b"b".to_vec(), b"2".to_vec())]
        ));
        for (parts, name) in [
            (&["EXISTS"][..], "exists"),
            (&["MGET"][..], "mget"),
            (&["SETNX", "k"][..], "setnx"),
            (&["SETNX", "k", "v", "x"][..], "setnx"),
            (&["MSET"][..], "mset"),
            (&["MSET", "a"][..], "mset"),
            (&["MSET", "a", "1", "b"][..], "mset"),
            (&["APPEND", "k"][..], "append"),
            (&["INCR"][..], "incr"),
            (&["INCR", "k", "1"][..], "incr"),
            (&["DECR"][..], "decr"),
        ] {
            assert!(
                matches!(parse(parts), Err(CommandError::WrongArity(n)) if n == name),
                "{parts:?}"
            );
        }
    }

    #[cfg(feature = "full")]
    #[test]
    fn is_write_matches_the_commands_that_change_data() {
        for parts in [
            &["SET", "k", "v"][..],
            &["DEL", "k"][..],
            &["EXPIRE", "k", "10"][..],
            &["PEXPIREAT", "k", "99999999999999"][..],
            &["PERSIST", "k"][..],
            &["LPUSH", "k", "x"][..],
            &["RPOP", "k"][..],
            &["HSET", "k", "f", "v"][..],
            &["HDEL", "k", "f"][..],
            &["SETNX", "k", "v"][..],
            &["MSET", "a", "1"][..],
            &["APPEND", "k", "v"][..],
            &["INCR", "k"][..],
            &["DECR", "k"][..],
        ] {
            assert!(parse(parts).unwrap().is_write(), "{parts:?}");
        }
        for parts in [
            &["GET", "k"][..],
            &["TTL", "k"][..],
            &["LRANGE", "k", "0", "-1"][..],
            &["HGETALL", "k"][..],
            &["DBSIZE"][..],
            &["REWRITEAOF"][..],
            &["EXISTS", "k"][..],
            &["MGET", "k"][..],
        ] {
            assert!(!parse(parts).unwrap().is_write(), "{parts:?}");
        }
    }

    #[cfg(feature = "full")]
    #[test]
    fn rewriteaof_is_not_executed_by_the_store() {
        let mut out = Vec::new();
        Command::Rewriteaof
            .execute(&Store::new())
            .encode(&mut out, &Protocol::Resp2);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "-ERR REWRITEAOF is handled by the AOF\r\n"
        );
    }

    #[cfg(not(feature = "full"))]
    #[test]
    fn full_only_commands_are_unknown() {
        for parts in [
            &["TTL", "k"][..],
            &["PTTL", "k"][..],
            &["EXPIRE", "k", "10"][..],
            &["PEXPIRE", "k", "10"][..],
            &["PEXPIREAT", "k", "10"][..],
            &["PERSIST", "k"][..],
            &["LPUSH", "k", "x"][..],
            &["RPUSH", "k", "x"][..],
            &["LPOP", "k"][..],
            &["RPOP", "k"][..],
            &["LLEN", "k"][..],
            &["LRANGE", "k", "0", "-1"][..],
            &["HSET", "k", "f", "v"][..],
            &["HGET", "k", "f"][..],
            &["HDEL", "k", "f"][..],
            &["HLEN", "k"][..],
            &["HEXISTS", "k", "f"][..],
            &["HGETALL", "k"][..],
            &["TYPE", "k"][..],
            &["DBSIZE"][..],
            &["REWRITEAOF"][..],
            &["EXISTS", "k"][..],
            &["SETNX", "k", "v"][..],
            &["MSET", "k", "v"][..],
            &["MGET", "k"][..],
            &["APPEND", "k", "v"][..],
            &["INCR", "k"][..],
            &["DECR", "k"][..],
        ] {
            assert!(
                matches!(parse(parts), Err(CommandError::UnknownCommand { .. })),
                "{parts:?}"
            );
        }
    }

    #[cfg(not(feature = "full"))]
    #[test]
    fn set_accepts_no_options() {
        assert!(matches!(parse(&["SET", "k", "v"]), Ok(Command::Set { .. })));
        for parts in [
            &["SET", "k", "v", "EX", "10"][..],
            &["SET", "k", "v", "PX", "10"][..],
            &["SET", "k", "v", "EX"][..],
            &["SET", "k"][..],
        ] {
            assert!(
                matches!(parse(parts), Err(CommandError::WrongArity("set"))),
                "{parts:?}"
            );
        }
    }

    #[cfg(not(feature = "full"))]
    #[test]
    fn only_set_and_del_are_writes() {
        assert!(parse(&["SET", "k", "v"]).unwrap().is_write());
        assert!(parse(&["DEL", "k"]).unwrap().is_write());
        assert!(!parse(&["GET", "k"]).unwrap().is_write());
        assert!(!parse(&["PING"]).unwrap().is_write());
    }
}

#[cfg(test)]
mod execute_tests {
    use super::*;

    fn run_with(store: &Store, parts: &[&str], protocol: &Protocol) -> String {
        let args: Vec<Vec<u8>> = parts.iter().map(|p| p.as_bytes().to_vec()).collect();
        let reply = match Command::from_args(&args) {
            Ok(cmd) => cmd.execute(store),
            Err(e) => Reply::from(e),
        };
        let mut out = Vec::new();
        reply.encode(&mut out, protocol);
        String::from_utf8(out).unwrap()
    }

    fn run(store: &Store, parts: &[&str]) -> String {
        run_with(store, parts, &Protocol::Resp2)
    }

    fn bulk(text: &str) -> String {
        format!("${}\r\n{}\r\n", text.len(), text)
    }

    #[cfg(feature = "full")]
    const WRONGTYPE: &str =
        "-WRONGTYPE Operation against a key holding the wrong kind of value\r\n";

    #[test]
    fn ping_replies_pong_or_echoes_the_message() {
        let store = Store::new();
        assert_eq!(run(&store, &["PING"]), "+PONG\r\n");
        assert_eq!(run(&store, &["PING", "hi"]), "$2\r\nhi\r\n");
    }

    #[test]
    fn get_and_set_replies() {
        let store = Store::new();
        assert_eq!(run(&store, &["GET", "k"]), "$-1\r\n");
        assert_eq!(run(&store, &["SET", "k", "v"]), "+OK\r\n");
        assert_eq!(run(&store, &["GET", "k"]), "$1\r\nv\r\n");
    }

    #[test]
    fn del_replies_with_the_number_of_removed_keys() {
        let store = Store::new();
        run(&store, &["SET", "a", "1"]);
        run(&store, &["SET", "b", "2"]);
        assert_eq!(run(&store, &["DEL", "a", "b", "missing"]), ":2\r\n");
        assert_eq!(run(&store, &["DEL", "a"]), ":0\r\n");
        #[cfg(feature = "full")]
        assert_eq!(run(&store, &["DBSIZE"]), ":0\r\n");
    }

    #[cfg(feature = "full")]
    #[test]
    fn ttl_replies_for_missing_persistent_and_expiring_keys() {
        let store = Store::new();
        assert_eq!(run(&store, &["TTL", "k"]), ":-2\r\n");
        assert_eq!(run(&store, &["PTTL", "k"]), ":-2\r\n");

        run(&store, &["SET", "k", "v"]);
        assert_eq!(run(&store, &["TTL", "k"]), ":-1\r\n");
        assert_eq!(run(&store, &["PTTL", "k"]), ":-1\r\n");

        run(&store, &["EXPIRE", "k", "100"]);
        assert_eq!(run(&store, &["TTL", "k"]), ":100\r\n");
        let pttl = run(&store, &["PTTL", "k"]);
        let ms: i64 = pttl.trim_start_matches(':').trim_end().parse().unwrap();
        assert!((99_000..=100_000).contains(&ms), "pttl was {ms}");
    }

    #[cfg(feature = "full")]
    #[test]
    fn expire_and_persist_reply_one_when_they_change_something() {
        let store = Store::new();
        assert_eq!(run(&store, &["EXPIRE", "k", "10"]), ":0\r\n");
        assert_eq!(run(&store, &["PERSIST", "k"]), ":0\r\n");

        run(&store, &["SET", "k", "v"]);
        assert_eq!(run(&store, &["PERSIST", "k"]), ":0\r\n");
        assert_eq!(run(&store, &["EXPIRE", "k", "10"]), ":1\r\n");
        assert_eq!(run(&store, &["PERSIST", "k"]), ":1\r\n");
        assert_eq!(run(&store, &["TTL", "k"]), ":-1\r\n");
    }

    #[cfg(feature = "full")]
    #[test]
    fn expire_with_a_non_positive_time_deletes_the_key() {
        let store = Store::new();
        run(&store, &["SET", "k", "v"]);
        assert_eq!(run(&store, &["EXPIRE", "k", "0"]), ":1\r\n");
        assert_eq!(run(&store, &["GET", "k"]), "$-1\r\n");
    }

    #[cfg(feature = "full")]
    #[test]
    fn list_replies() {
        let store = Store::new();
        assert_eq!(run(&store, &["LLEN", "l"]), ":0\r\n");
        assert_eq!(run(&store, &["LPOP", "l"]), "$-1\r\n");
        assert_eq!(run(&store, &["RPUSH", "l", "a", "b", "c"]), ":3\r\n");
        assert_eq!(run(&store, &["LPUSH", "l", "z"]), ":4\r\n");
        assert_eq!(run(&store, &["LLEN", "l"]), ":4\r\n");
        assert_eq!(run(&store, &["LPOP", "l"]), "$1\r\nz\r\n");
        assert_eq!(run(&store, &["RPOP", "l"]), "$1\r\nc\r\n");
    }

    #[cfg(feature = "full")]
    #[test]
    fn lrange_handles_negative_and_out_of_range_indexes() {
        let store = Store::new();
        run(&store, &["RPUSH", "l", "a", "b", "c"]);
        let abc = "*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n";
        assert_eq!(run(&store, &["LRANGE", "l", "0", "-1"]), abc);
        assert_eq!(run(&store, &["LRANGE", "l", "-100", "100"]), abc);
        assert_eq!(run(&store, &["LRANGE", "l", "1", "1"]), "*1\r\n$1\r\nb\r\n");
        assert_eq!(
            run(&store, &["LRANGE", "l", "-2", "-1"]),
            "*2\r\n$1\r\nb\r\n$1\r\nc\r\n"
        );
        assert_eq!(run(&store, &["LRANGE", "l", "5", "10"]), "*0\r\n");
        assert_eq!(run(&store, &["LRANGE", "l", "2", "0"]), "*0\r\n");
        assert_eq!(run(&store, &["LRANGE", "missing", "0", "-1"]), "*0\r\n");
    }

    #[cfg(feature = "full")]
    #[test]
    fn lrange_rejects_non_integer_indexes() {
        let store = Store::new();
        assert_eq!(
            run(&store, &["LRANGE", "l", "a", "1"]),
            "-ERR value is not an integer or out of range\r\n"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn hash_replies() {
        let store = Store::new();
        assert_eq!(run(&store, &["HGETALL", "h"]), "*0\r\n");
        assert_eq!(run(&store, &["HGET", "h", "f"]), "$-1\r\n");
        assert_eq!(run(&store, &["HLEN", "h"]), ":0\r\n");
        assert_eq!(run(&store, &["HEXISTS", "h", "f"]), ":0\r\n");

        assert_eq!(run(&store, &["HSET", "h", "f", "v"]), ":1\r\n");
        assert_eq!(run(&store, &["HSET", "h", "f", "w", "g", "x"]), ":1\r\n");
        assert_eq!(run(&store, &["HGET", "h", "f"]), "$1\r\nw\r\n");
        assert_eq!(run(&store, &["HLEN", "h"]), ":2\r\n");
        assert_eq!(run(&store, &["HEXISTS", "h", "f"]), ":1\r\n");
        assert_eq!(run(&store, &["HDEL", "h", "f", "nope"]), ":1\r\n");
        assert_eq!(
            run(&store, &["HGETALL", "h"]),
            "*2\r\n$1\r\ng\r\n$1\r\nx\r\n"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn type_reply_names_each_kind() {
        let store = Store::new();
        run(&store, &["SET", "s", "v"]);
        run(&store, &["RPUSH", "l", "a"]);
        run(&store, &["HSET", "h", "f", "v"]);
        assert_eq!(run(&store, &["TYPE", "s"]), "+string\r\n");
        assert_eq!(run(&store, &["TYPE", "l"]), "+list\r\n");
        assert_eq!(run(&store, &["TYPE", "h"]), "+hash\r\n");
        assert_eq!(run(&store, &["TYPE", "nope"]), "+none\r\n");
    }

    #[cfg(feature = "full")]
    #[test]
    fn using_the_wrong_type_replies_wrongtype() {
        let store = Store::new();
        run(&store, &["SET", "s", "v"]);
        run(&store, &["RPUSH", "l", "a"]);
        run(&store, &["HSET", "h", "f", "v"]);
        for parts in [
            &["LPUSH", "s", "x"][..],
            &["LLEN", "s"][..],
            &["HSET", "s", "f", "v"][..],
            &["HGETALL", "l"][..],
            &["GET", "l"][..],
            &["GET", "h"][..],
            &["RPOP", "h"][..],
        ] {
            assert_eq!(run(&store, parts), WRONGTYPE, "{parts:?}");
        }
    }

    #[cfg(feature = "full")]
    #[test]
    fn error_replies_match_redis_wording() {
        let store = Store::new();
        assert_eq!(
            run(&store, &["GET"]),
            "-ERR wrong number of arguments for 'get' command\r\n"
        );
        assert_eq!(
            run(&store, &["SET", "k", "v", "FOO", "1"]),
            "-ERR syntax error\r\n"
        );
        assert_eq!(
            run(&store, &["SET", "k", "v", "EX", "0"]),
            "-ERR invalid expire time in 'set' command\r\n"
        );
        assert_eq!(
            run(&store, &["SET", "k", "v", "EX", "x"]),
            "-ERR value is not an integer or out of range\r\n"
        );
        assert_eq!(
            run(&store, &["NOPE", "a"]),
            "-ERR unknown command 'NOPE', with args beginning with: 'a' \r\n"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn dbsize_counts_every_kind_of_key() {
        let store = Store::new();
        run(&store, &["SET", "s", "v"]);
        run(&store, &["RPUSH", "l", "a"]);
        run(&store, &["HSET", "h", "f", "v"]);
        assert_eq!(run(&store, &["DBSIZE"]), ":3\r\n");
    }

    #[test]
    fn nil_replies_depend_on_the_protocol() {
        let store = Store::new();
        assert_eq!(run(&store, &["GET", "k"]), "$-1\r\n");
        assert_eq!(run_with(&store, &["GET", "k"], &Protocol::Resp3), "_\r\n");
    }

    #[test]
    fn info_replies_with_the_requested_section() {
        let store = Store::new();
        let server = format!(
            "# Server\r\nminidis_version:{}\r\nredis_mode:standalone\r\n",
            env!("CARGO_PKG_VERSION")
        );
        let persistence = "# Persistence\r\nloading:0\r\n";
        assert_eq!(
            run(&store, &["INFO"]),
            bulk(&format!("{server}\r\n{persistence}"))
        );
        assert_eq!(run(&store, &["INFO", "server"]), bulk(&server));
        assert_eq!(run(&store, &["INFO", "PERSISTENCE"]), bulk(persistence));
        assert_eq!(run(&store, &["INFO", "nope"]), "$0\r\n\r\n");
    }

    #[test]
    fn command_replies_are_empty() {
        let store = Store::new();
        assert_eq!(run(&store, &["COMMAND"]), "*0\r\n");
        assert_eq!(run(&store, &["COMMAND", "DOCS"]), "*0\r\n");
        assert_eq!(run_with(&store, &["COMMAND"], &Protocol::Resp3), "*0\r\n");
        assert_eq!(
            run_with(&store, &["COMMAND", "DOCS"], &Protocol::Resp3),
            "%0\r\n"
        );
    }

    #[test]
    fn client_setinfo_replies_ok() {
        let store = Store::new();
        assert_eq!(
            run(&store, &["CLIENT", "SETINFO", "LIB-NAME", "redis-py"]),
            "+OK\r\n"
        );
        assert_eq!(
            run(&store, &["CLIENT", "SETINFO", "LIB-VER", "8.1.0"]),
            "+OK\r\n"
        );
    }

    #[test]
    fn hello_with_an_unsupported_version_replies_noproto() {
        let store = Store::new();
        for version in ["1", "4"] {
            assert_eq!(
                run(&store, &["HELLO", version]),
                "-NOPROTO unsupported protocol version\r\n"
            );
        }
    }

    #[cfg(feature = "full")]
    #[test]
    fn hgetall_replies_as_a_map_in_resp3() {
        let store = Store::new();
        run(&store, &["HSET", "h", "f", "v"]);
        assert_eq!(
            run(&store, &["HGETALL", "h"]),
            "*2\r\n$1\r\nf\r\n$1\r\nv\r\n"
        );
        assert_eq!(
            run_with(&store, &["HGETALL", "h"], &Protocol::Resp3),
            "%1\r\n$1\r\nf\r\n$1\r\nv\r\n"
        );
        assert_eq!(
            run_with(&store, &["HGETALL", "missing"], &Protocol::Resp3),
            "%0\r\n"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn string_command_replies() {
        let store = Store::new();
        assert_eq!(run(&store, &["SETNX", "a", "1"]), ":1\r\n");
        assert_eq!(run(&store, &["SETNX", "a", "2"]), ":0\r\n");
        assert_eq!(run(&store, &["MSET", "b", "2", "c", "3"]), "+OK\r\n");
        assert_eq!(run(&store, &["EXISTS", "a", "b", "missing", "a"]), ":3\r\n");
        assert_eq!(run(&store, &["APPEND", "a", "xy"]), ":3\r\n");
        assert_eq!(run(&store, &["GET", "a"]), "$3\r\n1xy\r\n");
        assert_eq!(run(&store, &["INCR", "n"]), ":1\r\n");
        assert_eq!(run(&store, &["DECR", "n"]), ":0\r\n");
        assert_eq!(run(&store, &["DECR", "n"]), ":-1\r\n");
        run(&store, &["RPUSH", "l", "x"]);
        assert_eq!(
            run(&store, &["MGET", "b", "missing", "l"]),
            "*3\r\n$1\r\n2\r\n$-1\r\n$-1\r\n"
        );
        assert_eq!(
            run_with(&store, &["MGET", "b", "missing"], &Protocol::Resp3),
            "*2\r\n$1\r\n2\r\n_\r\n"
        );
    }

    #[cfg(feature = "full")]
    #[test]
    fn string_command_errors_match_redis_wording() {
        let store = Store::new();
        run(&store, &["SET", "s", "abc"]);
        run(&store, &["SET", "max", "9223372036854775807"]);
        run(&store, &["RPUSH", "l", "x"]);
        assert_eq!(
            run(&store, &["INCR", "s"]),
            "-ERR value is not an integer or out of range\r\n"
        );
        assert_eq!(
            run(&store, &["INCR", "max"]),
            "-ERR increment or decrement would overflow\r\n"
        );
        assert_eq!(run(&store, &["INCR", "l"]), WRONGTYPE);
        assert_eq!(run(&store, &["APPEND", "l", "y"]), WRONGTYPE);
        assert_eq!(
            run(&store, &["MSET", "a"]),
            "-ERR wrong number of arguments for 'mset' command\r\n"
        );
    }

    #[cfg(not(feature = "full"))]
    #[test]
    fn full_only_commands_reply_with_redis_errors() {
        let store = Store::new();
        assert_eq!(
            run(&store, &["TTL", "k"]),
            "-ERR unknown command 'TTL', with args beginning with: 'k' \r\n"
        );
        assert_eq!(
            run(&store, &["SET", "k", "v", "EX", "10"]),
            "-ERR wrong number of arguments for 'set' command\r\n"
        );
    }
}
