use crate::store::StoreError;

#[derive(Debug)]
pub enum CommandError {
    UnknownCommand {
        name: String,
        args: String,
    },
    WrongArity(&'static str),
    #[cfg(feature = "full")]
    NotAnInteger,
    #[cfg(feature = "full")]
    InvalidExpire(&'static str),
    #[cfg(feature = "full")]
    SyntaxError,
    WrongType,
}
impl From<StoreError> for CommandError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::WrongType => CommandError::WrongType,
        }
    }
}

impl CommandError {
    pub(super) fn unknown_command(args: &[Vec<u8>]) -> CommandError {
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
