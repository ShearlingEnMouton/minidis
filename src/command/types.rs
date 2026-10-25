#[cfg(feature = "full")]
use std::time::Duration;

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
        #[cfg(feature = "full")]
        ttl: Option<Duration>,
    },
    Del {
        keys: Vec<Vec<u8>>,
    },
    #[cfg(feature = "full")]
    Ttl {
        key: Vec<u8>,
    },
    #[cfg(feature = "full")]
    Pttl {
        key: Vec<u8>,
    },
    #[cfg(feature = "full")]
    Expire {
        key: Vec<u8>,
        ttl: Duration,
    },
    #[cfg(feature = "full")]
    Persist {
        key: Vec<u8>,
    },
    #[cfg(feature = "full")]
    Dbsize {},
    #[cfg(feature = "full")]
    Rewriteaof,
    #[cfg(feature = "full")]
    Lpush {
        key: Vec<u8>,
        values: Vec<Vec<u8>>,
    },
    #[cfg(feature = "full")]
    Rpush {
        key: Vec<u8>,
        values: Vec<Vec<u8>>,
    },
    #[cfg(feature = "full")]
    Lpop {
        key: Vec<u8>,
    },
    #[cfg(feature = "full")]
    Rpop {
        key: Vec<u8>,
    },
    #[cfg(feature = "full")]
    Llen {
        key: Vec<u8>,
    },
    #[cfg(feature = "full")]
    Lrange {
        key: Vec<u8>,
        start: i64,
        stop: i64,
    },
    #[cfg(feature = "full")]
    Hset {
        key: Vec<u8>,
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
    },
    #[cfg(feature = "full")]
    Hget {
        key: Vec<u8>,
        field: Vec<u8>,
    },
    #[cfg(feature = "full")]
    Hdel {
        key: Vec<u8>,
        fields: Vec<Vec<u8>>,
    },
    #[cfg(feature = "full")]
    Hlen {
        key: Vec<u8>,
    },
    #[cfg(feature = "full")]
    Hexists {
        key: Vec<u8>,
        field: Vec<u8>,
    },
    #[cfg(feature = "full")]
    Hgetall {
        key: Vec<u8>,
    },
    #[cfg(feature = "full")]
    Type {
        key: Vec<u8>,
    },
}
