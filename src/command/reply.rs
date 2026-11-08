use super::CommandError;
use std::io::Write;

#[derive(Debug)]
pub enum Protocol {
    Resp2,
    Resp3,
}
pub enum Reply {
    Simple(&'static str),
    Error(String),
    Integer(i64),
    Bulk(Vec<u8>),
    Nil,
    Array(Vec<Reply>),
    Map(Vec<(Reply, Reply)>),
}
impl Reply {
    pub fn encode(&self, out: &mut Vec<u8>, protocol: &Protocol) {
        match self {
            Reply::Simple(s) => {
                out.extend_from_slice(b"+");
                out.extend_from_slice(s.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            Reply::Error(e) => {
                out.extend_from_slice(b"-");
                out.extend_from_slice(e.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            Reply::Integer(n) => {
                let _ = write!(out, ":{n}\r\n");
            }
            Reply::Bulk(v) => {
                let _ = write!(out, "${}\r\n", v.len());
                out.extend_from_slice(v);
                out.extend_from_slice(b"\r\n");
            }
            Reply::Nil => match protocol {
                Protocol::Resp2 => out.extend_from_slice(b"$-1\r\n"),
                Protocol::Resp3 => out.extend_from_slice(b"_\r\n"),
            },
            Reply::Array(items) => {
                let _ = write!(out, "*{}\r\n", items.len());
                for item in items {
                    item.encode(out, protocol);
                }
            }
            Reply::Map(pairs) => {
                let _ = match protocol {
                    Protocol::Resp2 => write!(out, "*{}\r\n", pairs.len() * 2),
                    Protocol::Resp3 => write!(out, "%{}\r\n", pairs.len()),
                };
                for (k, v) in pairs {
                    k.encode(out, protocol);
                    v.encode(out, protocol);
                }
            }
        }
    }
}
impl From<CommandError> for Reply {
    fn from(e: CommandError) -> Reply {
        match e {
            CommandError::WrongType => Reply::Error(
                "WRONGTYPE Operation against a key holding the wrong kind of value".to_owned(),
            ),
            CommandError::UnknownCommand { name, args } => Reply::Error(format!(
                "ERR unknown command '{name}', with args beginning with: {args}"
            )),
            CommandError::WrongArity(cmd) => {
                Reply::Error(format!("ERR wrong number of arguments for '{cmd}' command"))
            }
            CommandError::NotAnInteger => {
                Reply::Error("ERR value is not an integer or out of range".to_owned())
            }
            #[cfg(feature = "full")]
            CommandError::InvalidExpire(cmd) => {
                Reply::Error(format!("ERR invalid expire time in '{cmd}' command"))
            }
            CommandError::SyntaxError => Reply::Error("ERR syntax error".to_owned()),
            CommandError::NoProto => {
                Reply::Error("NOPROTO unsupported protocol version".to_owned())
            }
            CommandError::InvalidProtocolVersion => {
                Reply::Error("ERR Protocol version is not an integer or out of range".to_owned())
            }
            CommandError::HelloSyntax(option) => {
                Reply::Error(format!("ERR Syntax error in HELLO option '{option}'"))
            }
            CommandError::NoAuth => Reply::Error("NOAUTH Authentication required.".to_owned()),
            CommandError::HelloNoAuth => Reply::Error(
                "NOAUTH HELLO must be called with the client already authenticated, otherwise the HELLO <proto> AUTH <user> <pass> option can be used to authenticate the client and select the RESP protocol version at the same time".to_owned(),
            ),
            CommandError::WrongPass => Reply::Error(
                "WRONGPASS invalid username-password pair or user is disabled.".to_owned(),
            ),
            CommandError::AuthNotConfigured => Reply::Error(
                "ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?".to_owned(),
            ),
            #[cfg(feature = "full")]
            CommandError::Overflow => {
                Reply::Error("ERR increment or decrement would overflow".to_owned())
            }
            #[cfg(feature = "full")]
            CommandError::InvalidCursor => Reply::Error("ERR invalid cursor".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(reply: &Reply, protocol: &Protocol) -> String {
        let mut out = Vec::new();
        reply.encode(&mut out, protocol);
        String::from_utf8(out).unwrap()
    }

    fn sample_map() -> Reply {
        Reply::Map(vec![
            (Reply::Bulk(b"a".to_vec()), Reply::Integer(1)),
            (Reply::Bulk(b"b".to_vec()), Reply::Nil),
        ])
    }

    #[test]
    fn nil_encodes_per_protocol() {
        assert_eq!(encode(&Reply::Nil, &Protocol::Resp2), "$-1\r\n");
        assert_eq!(encode(&Reply::Nil, &Protocol::Resp3), "_\r\n");
    }

    #[test]
    fn scalar_replies_are_the_same_in_both_protocols() {
        for protocol in [Protocol::Resp2, Protocol::Resp3] {
            assert_eq!(encode(&Reply::Simple("OK"), &protocol), "+OK\r\n");
            assert_eq!(
                encode(&Reply::Error("ERR x".to_owned()), &protocol),
                "-ERR x\r\n"
            );
            assert_eq!(encode(&Reply::Integer(-7), &protocol), ":-7\r\n");
            assert_eq!(
                encode(&Reply::Bulk(b"ab".to_vec()), &protocol),
                "$2\r\nab\r\n"
            );
            let array = Reply::Array(vec![Reply::Integer(1), Reply::Bulk(b"a".to_vec())]);
            assert_eq!(encode(&array, &protocol), "*2\r\n:1\r\n$1\r\na\r\n");
        }
    }

    #[test]
    fn map_is_a_flat_array_in_resp2_and_a_map_in_resp3() {
        assert_eq!(
            encode(&sample_map(), &Protocol::Resp2),
            "*4\r\n$1\r\na\r\n:1\r\n$1\r\nb\r\n$-1\r\n"
        );
        assert_eq!(
            encode(&sample_map(), &Protocol::Resp3),
            "%2\r\n$1\r\na\r\n:1\r\n$1\r\nb\r\n_\r\n"
        );
    }

    #[test]
    fn nil_inside_an_array_follows_the_protocol() {
        let reply = Reply::Array(vec![Reply::Nil, Reply::Bulk(b"a".to_vec())]);
        assert_eq!(encode(&reply, &Protocol::Resp2), "*2\r\n$-1\r\n$1\r\na\r\n");
        assert_eq!(encode(&reply, &Protocol::Resp3), "*2\r\n_\r\n$1\r\na\r\n");
    }

    #[test]
    fn empty_array_and_empty_map() {
        assert_eq!(
            encode(&Reply::Array(Vec::new()), &Protocol::Resp3),
            "*0\r\n"
        );
        assert_eq!(encode(&Reply::Map(Vec::new()), &Protocol::Resp2), "*0\r\n");
        assert_eq!(encode(&Reply::Map(Vec::new()), &Protocol::Resp3), "%0\r\n");
    }
}
