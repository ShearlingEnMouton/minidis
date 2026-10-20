use super::CommandError;
use std::io::Write;

pub enum Reply {
    Simple(&'static str),
    Error(String),
    Integer(i64),
    Bulk(Vec<u8>),
    Nil,
    Array(Vec<Reply>),
}
impl Reply {
    pub fn encode(&self, out: &mut Vec<u8>) {
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
            Reply::Nil => out.extend_from_slice(b"$-1\r\n"),
            Reply::Array(items) => {
                let _ = write!(out, "*{}\r\n", items.len());
                for item in items {
                    item.encode(out);
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
            CommandError::InvalidExpire(cmd) => {
                Reply::Error(format!("ERR invalid expire time in '{cmd}' command"))
            }
            CommandError::SyntaxError => Reply::Error("ERR syntax error".to_owned()),
        }
    }
}
