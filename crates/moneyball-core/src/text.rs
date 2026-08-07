//! Char-boundary-safe string helpers.
//!
//! `&s[..n]` panics on multibyte text - lead names, error bodies, and
//! currency signs routinely contain it - so `clippy::string_slice` is
//! denied workspace-wide and every truncation goes through here.

/// Cut a string at (or just before) `cap` bytes, on a char boundary.
pub fn truncate_chars(s: &str, cap: usize) -> &str {
    let mut end = cap.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    #[allow(clippy::string_slice)] // end walked to a char boundary above
    &s[..end]
}

/// `truncate_chars`, appending `marker` only when something was cut.
pub fn truncate_marked(s: &str, cap: usize, marker: &str) -> String {
    let t = truncate_chars(s, cap);
    if t.len() == s.len() {
        s.to_string()
    } else {
        format!("{}{}", t, marker)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_on_char_boundary_not_byte() {
        // U+20B9 (rupee sign) is 3 bytes; cap lands mid-char.
        let s = "a\u{20B9}b";
        assert_eq!(truncate_chars(s, 2), "a");
        assert_eq!(truncate_chars(s, 4), "a\u{20B9}");
        assert_eq!(truncate_chars(s, 99), s);
    }

    #[test]
    fn marker_only_when_truncated() {
        assert_eq!(truncate_marked("hello", 3, "..."), "hel...");
        assert_eq!(truncate_marked("hi", 10, "..."), "hi");
    }
}
