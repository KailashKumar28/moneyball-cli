//! Char-boundary-safe string helpers, plain-text tables, base64 and URL
//! encoding.
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

/// Dependency-free base64 (standard alphabet, padded).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// `s` for a URL's query: every byte but RFC 3986's unreserved ones as `%XX`.
pub fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            b => format!("%{:02X}", b),
        })
        .collect()
}

/// Undo URL encoding on a query or form key/value (`%2B`, `+`).
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                // Decode from the byte pair, not a string slice: `%` followed
                // by multibyte text would split a char boundary and panic.
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A plain-text table: header, dashed rule, rows, each line ending "\n".
/// Widths count chars, as `{:<w$}` pads (names carry dashes like U+2013),
/// and a column whose every cell is a number ("1,234", "-12%") right-aligns.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let widths: Vec<usize> = (0..headers.len())
        .map(|i| {
            rows.iter()
                .map(|row| row[i].chars().count())
                .chain(std::iter::once(headers[i].chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let numeric: Vec<bool> = (0..headers.len())
        .map(|i| {
            rows.iter().all(|row| {
                row[i]
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | '-' | '+' | '%'))
            })
        })
        .collect();
    let line = |cells: Vec<&str>| {
        let padded: Vec<String> = cells
            .iter()
            .zip(&widths)
            .zip(&numeric)
            .map(|((c, w), num)| match num {
                true => format!("{:>w$}", c),
                false => format!("{:<w$}", c),
            })
            .collect();
        format!("{}\n", padded.join("  ").trim_end())
    };
    let mut out = line(headers.to_vec());
    out.push_str(&"-".repeat(widths.iter().sum::<usize>() + 2 * widths.len().saturating_sub(1)));
    out.push('\n');
    for row in rows {
        out.push_str(&line(row.iter().map(String::as_str).collect()));
    }
    out
}

/// A path under the home directory as `~/...`; anything else in full.
pub fn tilde(path: &std::path::Path) -> String {
    match crate::config::home_dir().and_then(|h| path.strip_prefix(h).ok().map(|r| r.to_path_buf()))
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_pads_by_chars_and_right_aligns_numbers() {
        let rows = vec![
            vec!["Primus \u{2013} K".to_string(), "115,387".to_string()],
            vec!["Lookalike".to_string(), "-".to_string()],
        ];
        assert_eq!(
            table(&["name", "spend"], &rows),
            "name          spend\n-------------------\nPrimus \u{2013} K  115,387\nLookalike         -\n"
        );
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"user:pa ss"), "dXNlcjpwYSBzcw==");
    }

    #[test]
    fn percent_encoding_round_trips() {
        let s = "https://x.dev/meta/callback?a=1&b=two words+\u{20B9}";
        let enc = percent_encode(s);
        assert!(
            enc.is_ascii() && !enc.contains(['/', ':', '&', '=', ' ', '+']),
            "{}",
            enc
        );
        assert_eq!(percent_decode(&enc), s);
        assert_eq!(percent_decode("a+b%2Bc%zz%"), "a b+c%zz%");
    }

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
