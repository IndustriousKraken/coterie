pub fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Truncate `s` to at most `max_chars` characters on a UTF-8 char
/// boundary, returning a borrowed sub-slice (no allocation). Unlike
/// `&s[..n]`, this never panics on multi-byte content.
pub fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_does_not_split_multibyte() {
        let multibyte = "é".repeat(200);
        let truncated = truncate_chars(multibyte.as_str(), 120);
        // Must cut on a char boundary (no panic) and keep exactly 120 chars.
        assert_eq!(truncated.chars().count(), 120);

        // ASCII shorter than the limit is returned unchanged.
        assert_eq!(truncate_chars("ascii", 100), "ascii");
    }
}
