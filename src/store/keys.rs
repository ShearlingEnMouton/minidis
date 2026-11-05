use super::Store;

impl Store {
    pub fn keys(&self, pattern: &[u8]) -> Vec<Vec<u8>> {
        todo!()
    }

    pub fn scan(&self, cursor: u64, pattern: Option<&[u8]>, count: usize) -> (u64, Vec<Vec<u8>>) {
        todo!()
    }
}

fn glob_match(pattern: &[u8], key: &[u8]) -> bool {
    let mut window: Option<(usize, usize)> = None;
    let (mut p, mut k): (usize, usize) = (0, 0);
    while k < key.len() {
        let matched: bool = match pattern.get(p) {
            None => false,
            Some(b'*') => {
                window = Some((p + 1, k));
                p += 1;
                true
            }
            Some(b'?') => {
                p += 1;
                k += 1;
                true
            }
            Some(b'\\') => match pattern.get(p + 1) {
                Some(&escaped) if escaped == key[k] => {
                    p += 2;
                    k += 1;
                    true
                }
                None if key[k] == b'\\' => {
                    p += 1;
                    k += 1;
                    true
                }
                _ => false,
            },
            Some(b'[') => {
                let (found, index) = match_class(pattern, p + 1, key[k]);
                if found {
                    p = index;
                    k += 1;
                }
                found
            }
            _ => {
                if pattern[p].eq(&key[k]) {
                    p += 1;
                    k += 1;
                    true
                } else {
                    false
                }
            }
        };

        if !matched {
            match window.as_mut() {
                Some(w) => {
                    w.1 += 1;
                    p = w.0;
                    k = w.1;
                }
                None => return false,
            }
        }
    }

    while pattern.get(p) == Some(&b'*') {
        p += 1;
    }
    p == pattern.len()
}

fn match_class(pattern: &[u8], start: usize, byte: u8) -> (bool, usize) {
    let mut i = start;
    let negated = pattern.get(i) == Some(&b'^');
    if negated {
        i += 1;
    }
    let mut found = false;

    loop {
        match pattern.get(i) {
            None => return (found != negated, i),
            Some(b']') => return (found != negated, i + 1),
            Some(b'\\') => match pattern.get(i + 1) {
                Some(&escaped) => {
                    found |= escaped == byte;
                    i += 2;
                }
                None => {
                    found |= byte == b'\\';
                    i += 1;
                }
            },
            Some(&c) => match (pattern.get(i + 1), pattern.get(i + 2)) {
                (Some(b'-'), Some(&end)) if end != b']' => {
                    let (lo, hi) = if c <= end { (c, end) } else { (end, c) };
                    found |= (lo..=hi).contains(&byte);
                    i += 3;
                }
                _ => {
                    found |= c == byte;
                    i += 1;
                }
            },
        }
    }
}
