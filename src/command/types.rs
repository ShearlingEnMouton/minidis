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
    Dbsize {},
    Rewriteaof,
    Lpush {
        key: Vec<u8>,
        values: Vec<Vec<u8>>,
    },
    Rpush {
        key: Vec<u8>,
        values: Vec<Vec<u8>>,
    },
    Lpop {
        key: Vec<u8>,
    },
    Rpop {
        key: Vec<u8>,
    },
    Llen {
        key: Vec<u8>,
    },
    Lrange {
        key: Vec<u8>,
        start: i64,
        stop: i64,
    },
    Hset {
        key: Vec<u8>,
        pairs: Vec<(Vec<u8>, Vec<u8>)>,
    },
    Hget {
        key: Vec<u8>,
        field: Vec<u8>,
    },
    Hdel {
        key: Vec<u8>,
        fields: Vec<Vec<u8>>,
    },
    Hlen {
        key: Vec<u8>,
    },
    Hexists {
        key: Vec<u8>,
        field: Vec<u8>,
    },
    Hgetall {
        key: Vec<u8>,
    },
    Type {
        key: Vec<u8>,
    },
}
