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
