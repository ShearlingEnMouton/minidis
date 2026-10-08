use crate::limits;

#[derive(Debug)]
pub enum ParseError {
    InvalidType,
    InvalidLength,
    InvalidTerminator,
    TooLarge,
}

type CommandArgs = Option<(Vec<Vec<u8>>, usize)>;

pub fn parse_command(buffer: &[u8]) -> Result<CommandArgs, ParseError> {
    let Some((count, mut pos)) = read_header(buffer, 0, b'*')? else {
        return Ok(None);
    };
    if count > limits::MAX_ARRAY_LEN {
        return Err(ParseError::TooLarge);
    }

    let mut args = Vec::with_capacity(count.min(16));

    for _ in 0..count {
        let Some((bulk_length, data_start)) = read_header(buffer, pos, b'$')? else {
            return Ok(None);
        };
        if bulk_length > limits::MAX_BULK_LEN {
            return Err(ParseError::TooLarge);
        }

        let data_end = data_start + bulk_length;
        if buffer.len() < data_end + 2 {
            return Ok(None);
        }
        if &buffer[data_end..data_end + 2] != b"\r\n" {
            return Err(ParseError::InvalidTerminator);
        }

        args.push(buffer[data_start..data_end].to_vec());
        pos = data_end + 2;
    }

    Ok(Some((args, pos)))
}

fn read_header(
    buffer: &[u8],
    pos: usize,
    prefix: u8,
) -> Result<Option<(usize, usize)>, ParseError> {
    let Some(&first) = buffer.get(pos) else {
        return Ok(None);
    };
    if first != prefix {
        return Err(ParseError::InvalidType);
    }

    let rest = &buffer[pos..];
    let window = &rest[..rest.len().min(limits::MAX_LINE_LEN + 2)];

    let Some(crlf) = find_crlf(window) else {
        if rest.len() >= limits::MAX_LINE_LEN + 2 {
            return Err(ParseError::TooLarge);
        }
        return Ok(None);
    };

    let digits = &rest[1..crlf];
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(ParseError::InvalidLength);
    }
    let value: usize = std::str::from_utf8(digits)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(ParseError::InvalidLength)?;

    Ok(Some((value, pos + crlf + 2)))
}

fn find_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\r\n")
}
