use super::Store;

impl Store {
    pub fn keys(&self, pattern: &[u8]) -> Vec<Vec<u8>> {
        let g = self.inner.lock().unwrap();
        let mut match_keys: Vec<Vec<u8>> = Vec::new();

        for (key, entry) in &g.data {
            if !entry.is_expired() && glob_match(pattern, key) {
                match_keys.push(key.clone());
            }
        }
        match_keys
    }

    pub fn scan(&self, cursor: u64, pattern: Option<&[u8]>, count: usize) -> (u64, Vec<Vec<u8>>) {
        let mut last_hash: u64 = 0;
        let mut catched_keys: Vec<Vec<u8>> = Vec::new();
        let mut counted: usize = 0;

        let g = self.inner.lock().unwrap();
        let mut iter = g.ordered_keys.range((cursor, Vec::new())..).peekable();
        while let Some((hash, key)) = iter.next() {
            counted += 1;
            last_hash = *hash;

            if g.data.get(key).is_some_and(|e| !e.is_expired()) {
                match pattern {
                    Some(p) => {
                        if glob_match(p, key) {
                            catched_keys.push(key.clone());
                        }
                    }
                    None => catched_keys.push(key.clone()),
                }
            }

            if counted >= count && iter.peek().is_none_or(|(next_hash, _)| *next_hash != *hash) {
                break;
            }
        }

        match iter.peek() {
            Some(_) => (last_hash.checked_add(1).unwrap_or(0), catched_keys),
            None => (0, catched_keys),
        }
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

#[cfg(test)]
mod tests {
    use super::glob_match;

    fn m(pattern: &str, key: &str) -> bool {
        glob_match(pattern.as_bytes(), key.as_bytes())
    }

    #[test]
    fn literals_and_question_marks() {
        assert!(m("", ""));
        assert!(!m("", "a"));
        assert!(m("abc", "abc"));
        assert!(!m("abc", "abd"));
        assert!(!m("abc", "ab"));
        assert!(!m("ab", "abc"));
        assert!(!m("A", "a"));
        assert!(m("h?llo", "hello"));
        assert!(!m("h?llo", "hllo"));
    }

    #[test]
    fn stars_match_any_run_of_bytes() {
        assert!(m("*", ""));
        assert!(m("*", "anything"));
        assert!(m("a*b", "axxb"));
        assert!(m("a*b", "ab"));
        assert!(!m("a*b", "ac"));
        assert!(m("ab*", "ab"));
        assert!(m("a**b", "ab"));
        assert!(m("*a", "ba"));
        assert!(!m("*a", "ab"));
        assert!(m("*x*y", "axbxcy"));
        assert!(m("*a*a*b", "aaaab"));
    }

    #[test]
    fn many_stars_do_not_backtrack_exponentially() {
        let key = "a".repeat(5000);
        assert!(!m("*a*a*a*a*a*a*a*a*a*a*b", &key));
        assert!(m("*a*a*a*a*a*a*a*a*a*a", &key));
    }

    #[test]
    fn classes() {
        assert!(m("[abc]", "b"));
        assert!(!m("[abc]", "d"));
        assert!(m("[a-c]", "b"));
        assert!(!m("[a-c]", "d"));
        assert!(m("[z-a]", "m"));
        assert!(!m("[^0-9]", "5"));
        assert!(m("[^0-9]", "x"));
        assert!(m("[a^]", "^"));
        assert!(m("[\\]]", "]"));
        assert!(m("[a-]", "-"));
        assert!(!m("[a-]", "b"));
        assert!(!m("[]", "a"));
        assert!(m("[^]", "a"));
        assert!(m("[a-c][0-9]", "b7"));
        assert!(m("user:[0-9]*", "user:42"));
    }

    #[test]
    fn an_unclosed_class_ends_at_the_end_of_the_pattern() {
        assert!(m("a[bc", "ab"));
        assert!(m("a[b-e", "ad"));
        assert!(!m("a[bc", "a[bc"));
    }

    #[test]
    fn backslash_escapes_the_next_byte() {
        assert!(m("a\\*b", "a*b"));
        assert!(!m("a\\*b", "axb"));
        assert!(m("\\?", "?"));
        assert!(!m("\\?", "x"));
        assert!(m("a\\\\b", "a\\b"));
        assert!(m("a\\", "a\\"));
        assert!(!m("a\\", "a"));
        assert!(!m("a\\", "ab"));
    }

    #[test]
    fn a_trailing_backslash_inside_a_class_is_literal() {
        assert!(m("[a\\", "\\"));
        assert!(m("[a\\", "a"));
        assert!(!m("[a\\", "b"));
    }
}
