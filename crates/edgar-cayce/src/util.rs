//! Small string helpers with one job: never slice a `&str` on a
//! non-char boundary. Several ask-path truncations used `&s[..N]` on
//! LLM/corpus text, which panics on multi-byte UTF-8. Use these instead.

/// Largest byte index ≤ `max` that is a char boundary.
pub fn floor_char_boundary(s: &str, max: usize) -> usize {
    let mut i = max.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `s` shortened to at most `max` bytes, ending on a char boundary.
pub fn truncate(s: &str, max: usize) -> &str {
    &s[..floor_char_boundary(s, max)]
}

/// Split off a head of at most `max` bytes: char-boundary safe, preferring
/// a trailing newline then space. Returns `(head, rest)`; head is never
/// empty unless `s` is empty.
pub fn split_head(s: &str, max: usize) -> (&str, &str) {
    if s.len() <= max {
        return (s, "");
    }
    let end = floor_char_boundary(s, max);
    let split_at = s[..end]
        .rfind('\n')
        .map(|i| i + 1)
        .or_else(|| s[..end].rfind(' ').map(|i| i + 1))
        .filter(|&i| i > 0)
        .unwrap_or(end);
    let split_at = split_at.min(s.len());
    let split_at = if s.is_char_boundary(split_at) {
        split_at
    } else {
        end
    };
    (&s[..split_at], &s[split_at..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multibyte_safe() {
        let s = "héllo wörld — test";
        assert!(truncate(s, 5).len() <= 5);
        assert!(truncate(s, 5).chars().count() > 0);
        let (h, r) = split_head(s, 5);
        assert_eq!(format!("{h}{r}"), s);
        assert!(!h.is_empty());
        let (h, r) = split_head("a\nb\nc", 3);
        assert_eq!((h, r), ("a\n", "b\nc"));
    }
}
