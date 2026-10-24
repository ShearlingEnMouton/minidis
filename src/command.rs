mod error;
mod reply;
mod types;

use crate::store::{Store, TtlStatus};
use error::CommandError;
pub use reply::Reply;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
pub use types::Command;

impl Command {
    fn parse_int(arg: &[u8]) -> Result<i64, CommandError> {
        let s = std::str::from_utf8(arg).map_err(|_| CommandError::NotAnInteger)?;
        s.parse().map_err(|_| CommandError::NotAnInteger)
    }
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
    fn calculate_remaining(expire_at: u64, unit: fn(u64) -> Duration) -> Duration {
        (UNIX_EPOCH + unit(expire_at))
            .duration_since(SystemTime::now())
            .unwrap_or_default()
    }
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
            b"PEXPIREAT" => match args.len() {
                3 => {
                    Self::expire_at_command(&args[1], &args[2], Duration::from_millis, "pexpireat")
                }
                _ => Err(CommandError::WrongArity("pexpireat")),
            },
            b"PERSIST" => match args.len() {
                2 => Ok(Command::Persist {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("persist")),
            },
            b"DBSIZE" => match args.len() {
                1 => Ok(Command::Dbsize {}),
                _ => Err(CommandError::WrongArity("dbsize")),
            },
            b"LPUSH" => match args.len() {
                3.. => Ok(Command::Lpush {
                    key: args[1].clone(),
                    values: args[2..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity("lpush")),
            },
            b"RPUSH" => match args.len() {
                3.. => Ok(Command::Rpush {
                    key: args[1].clone(),
                    values: args[2..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity("rpush")),
            },
            b"LPOP" => match args.len() {
                2 => Ok(Command::Lpop {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("lpop")),
            },
            b"RPOP" => match args.len() {
                2 => Ok(Command::Rpop {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("rpop")),
            },
            b"LLEN" => match args.len() {
                2 => Ok(Command::Llen {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("llen")),
            },
            b"LRANGE" => match args.len() {
                4 => Ok(Command::Lrange {
                    key: args[1].clone(),
                    start: Self::parse_int(&args[2])?,
                    stop: Self::parse_int(&args[3])?,
                }),
                _ => Err(CommandError::WrongArity("lrange")),
            },
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
            b"HGET" => match args.len() {
                3 => Ok(Command::Hget {
                    key: args[1].clone(),
                    field: args[2].clone(),
                }),
                _ => Err(CommandError::WrongArity("hget")),
            },
            b"HDEL" => match args.len() {
                3.. => Ok(Command::Hdel {
                    key: args[1].clone(),
                    fields: args[2..].to_vec(),
                }),
                _ => Err(CommandError::WrongArity("hdel")),
            },
            b"HLEN" => match args.len() {
                2 => Ok(Command::Hlen {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("hlen")),
            },
            b"HEXISTS" => match args.len() {
                3 => Ok(Command::Hexists {
                    key: args[1].clone(),
                    field: args[2].clone(),
                }),
                _ => Err(CommandError::WrongArity("hexists")),
            },
            b"HGETALL" => match args.len() {
                2 => Ok(Command::Hgetall {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("hgetall")),
            },
            b"TYPE" => match args.len() {
                2 => Ok(Command::Type {
                    key: args[1].clone(),
                }),
                _ => Err(CommandError::WrongArity("type")),
            },
            _ => Err(CommandError::unknown_command(args)),
        }
    }

    pub fn execute(self, store: &Store) -> Reply {
        match self {
            Command::Ping { message: None } => Reply::Simple("PONG"),
            Command::Ping {
                message: Some(message),
            } => Reply::Bulk(message),
            Command::Get { key } => match store.get(&key) {
                Ok(None) => Reply::Nil,
                Ok(Some(value)) => Reply::Bulk(value),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Set { key, value, ttl } => {
                store.set(key, value, ttl);
                Reply::Simple("OK")
            }
            Command::Del { keys } => {
                let removed = store.del(&keys);
                Reply::Integer(removed as i64)
            }
            Command::Ttl { key } => match store.ttl(&key) {
                TtlStatus::NoKey => Reply::Integer(-2),
                TtlStatus::NoExpiry => Reply::Integer(-1),
                TtlStatus::Remaining(d) => Reply::Integer(((d.as_millis() + 500) / 1000) as i64),
            },
            Command::Pttl { key } => match store.ttl(&key) {
                TtlStatus::NoKey => Reply::Integer(-2),
                TtlStatus::NoExpiry => Reply::Integer(-1),
                TtlStatus::Remaining(d) => Reply::Integer(d.as_millis() as i64),
            },
            Command::Expire { key, ttl } => {
                let result = store.expire(&key, ttl);
                Reply::Integer(result.into())
            }
            Command::Persist { key } => {
                let result = store.persist(&key);
                Reply::Integer(result.into())
            }
            Command::Dbsize {} => Reply::Integer(store.dbsize() as i64),
            Command::Lpush { key, values } => match store.lpush(key, values) {
                Ok(len) => Reply::Integer(len as i64),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Rpush { key, values } => match store.rpush(key, values) {
                Ok(len) => Reply::Integer(len as i64),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Lpop { key } => match store.lpop(&key) {
                Ok(None) => Reply::Nil,
                Ok(Some(v)) => Reply::Bulk(v),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Rpop { key } => match store.rpop(&key) {
                Ok(None) => Reply::Nil,
                Ok(Some(v)) => Reply::Bulk(v),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Llen { key } => match store.llen(&key) {
                Ok(len) => Reply::Integer(len as i64),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Lrange { key, start, stop } => match store.lrange(&key, start, stop) {
                Ok(items) => Reply::Array(items.into_iter().map(Reply::Bulk).collect()),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Hset { key, pairs } => match store.hset(key, pairs) {
                Ok(added) => Reply::Integer(added as i64),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Hget { key, field } => match store.hget(&key, &field) {
                Ok(None) => Reply::Nil,
                Ok(Some(v)) => Reply::Bulk(v),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Hdel { key, fields } => match store.hdel(&key, &fields) {
                Ok(removed) => Reply::Integer(removed as i64),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Hlen { key } => match store.hlen(&key) {
                Ok(len) => Reply::Integer(len as i64),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Hexists { key, field } => match store.hexists(&key, &field) {
                Ok(exists) => Reply::Integer(exists.into()),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Hgetall { key } => match store.hgetall(&key) {
                Ok(pairs) => Reply::Array(
                    pairs
                        .into_iter()
                        .flat_map(|(f, v)| [Reply::Bulk(f), Reply::Bulk(v)])
                        .collect(),
                ),
                Err(e) => CommandError::from(e).into(),
            },
            Command::Type { key } => Reply::Simple(store.key_type(&key)),
        }
    }

    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Command::Set { .. }
                | Command::Del { .. }
                | Command::Expire { .. }
                | Command::Persist { .. }
                | Command::Lpush { .. }
                | Command::Rpush { .. }
                | Command::Lpop { .. }
                | Command::Rpop { .. }
                | Command::Hset { .. }
                | Command::Hdel { .. }
        )
    }
}
