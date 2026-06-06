//! Shared CSV-writing helper for admin exports. Hand-rolled (no
//! `csv` crate dependency) per the project's "no new dep" preference.
//! Always quotes — simpler than deciding when to — and escapes
//! embedded quotes per RFC 4180.

/// Emit a single CSV field into `out`. Always wraps the value in
/// double quotes and doubles any internal `"` so the field is safe
/// regardless of commas, quotes, or newlines inside it.
pub fn push_csv(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        if c == '"' {
            out.push('"');
            out.push('"');
        } else {
            out.push(c);
        }
    }
    out.push('"');
}

/// Emit a single CSV field that originates from member-controlled free
/// text, additionally neutralizing spreadsheet formula injection
/// (CWE-1236). Behaves exactly like [`push_csv`] (always quotes; doubles
/// internal `"`), but when `value`'s first character is a formula
/// trigger (`=`, `+`, `-`, `@`) or a control character a spreadsheet may
/// treat as starting a formula (TAB, CR, LF), it emits a single quote
/// (`'`) immediately after the opening `"` so the cell renders as
/// literal text rather than being evaluated. An empty value stays
/// empty-quoted (`""`). Use [`push_csv`] for server-controlled columns
/// (numbers, dates, enums, booleans) so they are not altered.
pub fn push_csv_user(out: &mut String, value: &str) {
    out.push('"');
    if let Some(first) = value.chars().next() {
        if matches!(first, '=' | '+' | '-' | '@' | '\t' | '\r' | '\n') {
            out.push('\'');
        }
    }
    for c in value.chars() {
        if c == '"' {
            out.push('"');
            out.push('"');
        } else {
            out.push(c);
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_csv_user_neutralizes_leading_equals() {
        let mut out = String::new();
        push_csv_user(&mut out, r#"=HYPERLINK("x","y")"#);
        assert!(
            out.starts_with(r#""'=HYPERLINK"#),
            "expected a neutralizing single quote directly after the opening quote, got {out}",
        );
    }

    #[test]
    fn push_csv_user_neutralizes_plus_minus_at_and_controls() {
        for input in ["+1", "-1", "@x", "\tdanger"] {
            let mut out = String::new();
            push_csv_user(&mut out, input);
            assert!(
                out.starts_with("\"'"),
                "expected a neutralizing single quote for input {input:?}, got {out}",
            );
        }
    }

    #[test]
    fn push_csv_user_leaves_ordinary_values_unquoted_inside() {
        let mut out = String::new();
        push_csv_user(&mut out, "O'Brien, Sean");
        // RFC 4180 quoting only — no `'` injected after the opening quote,
        // and the internal apostrophe is preserved unchanged.
        assert_eq!(out, r#""O'Brien, Sean""#);
    }

    #[test]
    fn push_csv_user_doubles_internal_quotes() {
        let mut out = String::new();
        push_csv_user(&mut out, r#"say "hi""#);
        assert_eq!(out, r#""say ""hi""""#);
    }
}
