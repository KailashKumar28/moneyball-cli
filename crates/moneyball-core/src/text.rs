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

/// 1234567 -> "1,234,567": the one number format every surface shares.
pub fn commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "1 lead", "4 leads": a count with its noun agreeing (client-facing
/// text never says "lead(s)" or "1 leads").
pub fn count(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", n, if n == 1 { one } else { many })
}

/// Split a pre-formatted line into pieces of at most `width` columns
/// (chars), the first at `width` and the rest at `rest`. Keeps every
/// internal space, so aligned columns survive (word-wrapping collapses
/// them), and loses nothing (clipping drops the end, often the path or the
/// reason). Breaks at the last space that fits, else mid-token (a path).
pub fn wrap_columns(s: &str, width: usize, rest: usize) -> Vec<String> {
    let mut left: Vec<char> = s.trim_end().chars().collect();
    let mut out = Vec::new();
    let mut w = width.max(1);
    while left.len() > w {
        let cut = match left[1..=w].iter().rposition(|c| *c == ' ') {
            Some(p) => p + 1,
            None => w,
        };
        out.push(
            left[..cut]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string(),
        );
        left = left[cut..]
            .iter()
            .skip_while(|c| **c == ' ')
            .copied()
            .collect();
        w = rest.max(1);
    }
    if !left.is_empty() || out.is_empty() {
        out.push(left.into_iter().collect());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_without_losing_or_collapsing() {
        // Short lines and their internal space runs come back untouched.
        assert_eq!(wrap_columns("a    b  ", 20, 18), vec!["a    b"]);
        assert_eq!(wrap_columns("", 20, 18), vec![""]);
        // Prose breaks at a space; a path with none breaks mid-token.
        let prose = "fix: moneyball crm secret leadzump_token";
        let pieces = wrap_columns(prose, 20, 18);
        assert_eq!(
            pieces,
            vec!["fix: moneyball crm", "secret", "leadzump_token"]
        );
        let path = "report: /a/very/long/path/to/Fincity-Daily-\u{20B9}.html";
        let pieces = wrap_columns(path, 16, 14);
        assert!(pieces[0].chars().count() <= 16);
        assert!(pieces[1..].iter().all(|p| p.chars().count() <= 14));
        assert_eq!(pieces.concat().replace(' ', ""), path.replace(' ', ""));
    }

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
