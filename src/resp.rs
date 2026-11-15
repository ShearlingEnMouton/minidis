use crate::limits;

#[derive(Debug)]
pub enum ParseError {
    InvalidType,
    InvalidLength,
    InvalidTerminator,
    TooLarge,
    UnbalancedQuotes,
    InlineLineTooLong,
}

type CommandArgs = Option<(Vec<Vec<u8>>, usize)>;

pub fn parse_inline_command(buffer: &[u8]) -> Result<CommandArgs, ParseError> {
    match buffer.iter().position(|&b| b == b'\n') {
        Some(i) => {
            let line = buffer[..i].strip_suffix(b"\r").unwrap_or(&buffer[..i]);
            let args = split_args(line)?;
            Ok(Some((args, i + 1)))
        }
        None => {
            if buffer.len() > limits::MAX_INLINE_LEN {
                return Err(ParseError::InlineLineTooLong);
            }
            Ok(None)
        }
    }
}
fn split_args(args: &[u8]) -> Result<Vec<Vec<u8>>, ParseError> {
    let mut result = Vec::new();
    let mut i = 0;

    loop {
        while args.get(i).is_some_and(|&b| is_space(b)) {
            i += 1;
        }
        if i >= args.len() {
            return Ok(result);
        }

        let mut current = Vec::new();
        let mut in_double = false;
        let mut in_single = false;

        loop {
            let byte = args.get(i).copied();
            if in_double {
                match byte {
                    None => return Err(ParseError::UnbalancedQuotes),
                    Some(b'\\') => match (
                        args.get(i + 1),
                        hex_value(args.get(i + 2)),
                        hex_value(args.get(i + 3)),
                    ) {
                        (Some(b'x'), Some(high), Some(low)) => {
                            current.push(high * 16 + low);
                            i += 4;
                        }
                        (Some(&next), _, _) => {
                            current.push(match next {
                                b'n' => b'\n',
                                b'r' => b'\r',
                                b't' => b'\t',
                                b'b' => 0x08,
                                b'a' => 0x07,
                                other => other,
                            });
                            i += 2;
                        }
                        (None, _, _) => return Err(ParseError::UnbalancedQuotes),
                    },
                    Some(b'"') => {
                        if args.get(i + 1).is_some_and(|&b| !is_space(b)) {
                            return Err(ParseError::UnbalancedQuotes);
                        }
                        i += 1;
                        break;
                    }
                    Some(b) => {
                        current.push(b);
                        i += 1;
                    }
                }
            } else if in_single {
                match byte {
                    None => return Err(ParseError::UnbalancedQuotes),
                    Some(b'\\') if args.get(i + 1) == Some(&b'\'') => {
                        current.push(b'\'');
                        i += 2;
                    }
                    Some(b'\'') => {
                        if args.get(i + 1).is_some_and(|&b| !is_space(b)) {
                            return Err(ParseError::UnbalancedQuotes);
                        }
                        i += 1;
                        break;
                    }
                    Some(b) => {
                        current.push(b);
                        i += 1;
                    }
                }
            } else {
                match byte {
                    None => break,
                    Some(b) if is_space(b) => break,
                    Some(b'"') => {
                        in_double = true;
                        i += 1;
                    }
                    Some(b'\'') => {
                        in_single = true;
                        i += 1;
                    }
                    Some(b) => {
                        current.push(b);
                        i += 1;
                    }
                }
            }
        }

        result.push(current);
    }
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn hex_value(byte: Option<&u8>) -> Option<u8> {
    byte.and_then(|&b| (b as char).to_digit(16))
        .map(|d| d as u8)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    const SET_A: &[u8] = b"*3\r\n$3\r\nSET\r\n$1\r\na\r\n$1\r\n1\r\n";
    const GET_A: &[u8] = b"*2\r\n$3\r\nGET\r\n$1\r\na\r\n";

    fn words(parts: &[&str]) -> Vec<Vec<u8>> {
        parts.iter().map(|p| p.as_bytes().to_vec()).collect()
    }

    #[test]
    fn parses_a_complete_command() {
        let (args, consumed) = parse_command(SET_A).unwrap().unwrap();
        assert_eq!(args, words(&["SET", "a", "1"]));
        assert_eq!(consumed, SET_A.len());
    }

    #[test]
    fn consumed_counts_only_the_first_of_two_pipelined_commands() {
        let buffer = [SET_A, GET_A].concat();

        let (args, consumed) = parse_command(&buffer).unwrap().unwrap();
        assert_eq!(args, words(&["SET", "a", "1"]));
        assert_eq!(consumed, SET_A.len());

        let (args, consumed) = parse_command(&buffer[consumed..]).unwrap().unwrap();
        assert_eq!(args, words(&["GET", "a"]));
        assert_eq!(consumed, GET_A.len());
    }

    #[test]
    fn bytes_after_the_first_command_are_left_alone() {
        let buffer = [SET_A, b"*2\r\n$3\r\nGE"].concat();

        let (_, consumed) = parse_command(&buffer).unwrap().unwrap();

        assert_eq!(consumed, SET_A.len());
        assert!(parse_command(&buffer[consumed..]).unwrap().is_none());
    }

    #[test]
    fn every_incomplete_prefix_asks_for_more_data() {
        for end in 0..SET_A.len() {
            assert!(
                matches!(parse_command(&SET_A[..end]), Ok(None)),
                "prefix of {end} bytes"
            );
        }
    }

    #[test]
    fn empty_array_consumes_its_header() {
        let (args, consumed) = parse_command(b"*0\r\n").unwrap().unwrap();
        assert!(args.is_empty());
        assert_eq!(consumed, 4);
    }

    #[test]
    fn bulk_strings_may_contain_line_breaks() {
        let input = b"*1\r\n$4\r\na\r\nb\r\n";
        let (args, consumed) = parse_command(input).unwrap().unwrap();
        assert_eq!(args, vec![b"a\r\nb".to_vec()]);
        assert_eq!(consumed, input.len());
    }

    #[test]
    fn empty_bulk_string_is_an_empty_word() {
        let (args, _) = parse_command(b"*1\r\n$0\r\n\r\n").unwrap().unwrap();
        assert_eq!(args, vec![Vec::<u8>::new()]);
    }

    #[test]
    fn rejects_an_unexpected_first_byte() {
        assert!(matches!(
            parse_command(b"?abc\r\n"),
            Err(ParseError::InvalidType)
        ));
        assert!(matches!(
            parse_command(b"PING\r\n"),
            Err(ParseError::InvalidType)
        ));
    }

    #[test]
    fn rejects_a_word_that_is_not_a_bulk_string() {
        assert!(matches!(
            parse_command(b"*1\r\n+abc\r\n"),
            Err(ParseError::InvalidType)
        ));
    }

    #[test]
    fn rejects_a_word_without_the_line_break_after_it() {
        assert!(matches!(
            parse_command(b"*1\r\n$3\r\nabcXY"),
            Err(ParseError::InvalidTerminator)
        ));
    }

    #[test]
    fn rejects_non_numeric_lengths() {
        for input in [&b"*abc\r\n"[..], b"*\r\n", b"*-1\r\n", b"*1\r\n$x\r\n"] {
            assert!(
                matches!(parse_command(input), Err(ParseError::InvalidLength)),
                "{:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn rejects_an_array_longer_than_the_limit() {
        let input = format!("*{}\r\n", limits::MAX_ARRAY_LEN + 1);
        assert!(matches!(
            parse_command(input.as_bytes()),
            Err(ParseError::TooLarge)
        ));
    }

    #[test]
    fn accepts_an_array_header_at_the_limit() {
        let input = format!("*{}\r\n", limits::MAX_ARRAY_LEN);
        assert!(matches!(parse_command(input.as_bytes()), Ok(None)));
    }

    #[test]
    fn rejects_a_bulk_string_longer_than_the_limit() {
        let input = format!("*1\r\n${}\r\n", limits::MAX_BULK_LEN + 1);
        assert!(matches!(
            parse_command(input.as_bytes()),
            Err(ParseError::TooLarge)
        ));
    }

    #[test]
    fn rejects_a_header_line_that_never_ends() {
        let input = format!("*{}", "1".repeat(limits::MAX_LINE_LEN + 10));
        assert!(matches!(
            parse_command(input.as_bytes()),
            Err(ParseError::TooLarge)
        ));
    }

    fn split(line: &[u8]) -> Option<Vec<Vec<u8>>> {
        split_args(line).ok()
    }

    fn bytes(parts: &[&[u8]]) -> Option<Vec<Vec<u8>>> {
        Some(parts.iter().map(|p| p.to_vec()).collect())
    }

    #[test]
    fn inline_arguments_are_separated_by_spaces() {
        assert_eq!(split(b"SET k v"), bytes(&[b"SET", b"k", b"v"]));
        assert_eq!(split(b"  GET \t k  "), bytes(&[b"GET", b"k"]));
        assert_eq!(split(b""), bytes(&[]));
        assert_eq!(split(b" \t "), bytes(&[]));
        assert_eq!(split(b"a\\nb"), bytes(&[b"a\\nb"]));
    }

    #[test]
    fn double_quotes_keep_spaces_and_read_escapes() {
        assert_eq!(split(b"SET k \"a b\""), bytes(&[b"SET", b"k", b"a b"]));
        assert_eq!(split(b"\"\""), bytes(&[b""]));
        assert_eq!(split(b"\"\\x41\\x7a\""), bytes(&[b"Az"]));
        assert_eq!(split(b"\"\\xZZ\""), bytes(&[b"xZZ"]));
        assert_eq!(split(b"\"\\n\\r\\t\\b\\a\""), bytes(&[b"\n\r\t\x08\x07"]));
        assert_eq!(split(b"\"a\\\"b\\\\\""), bytes(&[b"a\"b\\"]));
        assert_eq!(split(b"a\"b c\"d"), None);
        assert_eq!(split(b"a\"b c\""), bytes(&[b"ab c"]));
    }

    #[test]
    fn single_quotes_only_read_an_escaped_quote() {
        assert_eq!(split(b"'it\\'s'"), bytes(&[b"it's"]));
        assert_eq!(split(b"'a\\nb'"), bytes(&[b"a\\nb"]));
        assert_eq!(split(b"''"), bytes(&[b""]));
    }

    #[test]
    fn unbalanced_quotes_are_rejected() {
        for line in [
            &b"GET \"abc"[..],
            b"GET 'abc",
            b"GET \"a\"b",
            b"GET 'a'b",
            b"GET \"abc\\",
        ] {
            assert!(
                matches!(split_args(line), Err(ParseError::UnbalancedQuotes)),
                "{}",
                String::from_utf8_lossy(line)
            );
        }
    }

    #[test]
    fn an_inline_command_ends_at_a_newline() {
        let (args, consumed) = parse_inline_command(b"PING\r\nGE").unwrap().unwrap();
        assert_eq!(args, words(&["PING"]));
        assert_eq!(consumed, 6);

        let (args, consumed) = parse_inline_command(b"GET k\n").unwrap().unwrap();
        assert_eq!(args, words(&["GET", "k"]));
        assert_eq!(consumed, 6);

        let (args, consumed) = parse_inline_command(b"\r\n").unwrap().unwrap();
        assert!(args.is_empty());
        assert_eq!(consumed, 2);
    }

    #[test]
    fn an_unfinished_inline_line_asks_for_more_data() {
        assert!(parse_inline_command(b"").unwrap().is_none());
        assert!(parse_inline_command(b"GET k").unwrap().is_none());
        let at_limit = vec![b'a'; limits::MAX_INLINE_LEN];
        assert!(parse_inline_command(&at_limit).unwrap().is_none());
    }

    #[test]
    fn an_inline_line_over_the_limit_is_rejected() {
        let too_long = vec![b'a'; limits::MAX_INLINE_LEN + 1];
        assert!(matches!(
            parse_inline_command(&too_long),
            Err(ParseError::InlineLineTooLong)
        ));
    }

    #[test]
    fn an_inline_quote_error_is_reported() {
        assert!(matches!(
            parse_inline_command(b"GET \"k\r\n"),
            Err(ParseError::UnbalancedQuotes)
        ));
    }
}
