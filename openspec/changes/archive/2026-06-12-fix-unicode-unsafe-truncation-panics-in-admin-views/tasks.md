## 1. Add a char-boundary-safe truncate helper

- [x] 1.1 In `src/util/string.rs`, add a public function:
  ```rust
  /// Truncate `s` to at most `max_chars` characters on a UTF-8 char
  /// boundary, returning a borrowed sub-slice (no allocation). Unlike
  /// `&s[..n]`, this never panics on multi-byte content.
  pub fn truncate_chars(s: &str, max_chars: usize) -> &str {
      match s.char_indices().nth(max_chars) {
          Some((idx, _)) => &s[..idx],
          None => s,
      }
  }
  ```
- [x] 1.2 Add a unit test in `src/util/string.rs` named `truncate_chars_does_not_split_multibyte` asserting that `truncate_chars("é".repeat(200).as_str(), 120)` returns a valid `&str` (no panic) whose `chars().count()` is `120`, and that `truncate_chars("ascii", 100)` returns `"ascii"` unchanged.

## 2. Fix the audit-log truncate panic

- [x] 2.1 In `src/web/portal/admin/audit.rs`, replace the body of `fn truncate(s: &str) -> &str` (currently `if s.len() <= MAX { s } else { &s[..MAX] }`, lines ~139-146) so it no longer slices by byte index. Either call the new helper:
  ```rust
  fn truncate(s: &str) -> &str {
      crate::util::string::truncate_chars(s, 120)
  }
  ```
  or inline the equivalent `char_indices().nth(120)` match. Keep the `&str` return type so `format_detail` is unaffected.
- [x] 2.2 Add a regression test (in `src/web/portal/admin/audit.rs` tests, or `tests/`) named `audit_truncate_handles_multibyte_boundary` that calls `truncate` (or `format_detail` with a `new` value of `"é".repeat(200)`) and asserts it returns without panicking.

## 3. Fix the announcements preview panic

- [x] 3.1 In `src/web/portal/admin/announcements.rs` (the `content_preview` block around lines 230-234), replace `&a.content[..100]` with the char-safe helper. Append the `...` ellipsis only when truncation actually happened:
  ```rust
  let preview = crate::util::string::truncate_chars(&a.content, 100);
  let content_preview = if preview.len() < a.content.len() {
      format!("{}...", preview)
  } else {
      a.content.clone()
  };
  ```
- [x] 3.2 Confirm no remaining `&a.content[..` byte-index slice exists in the function after the edit.

## 4. Verify

- [x] 4.1 Run `cargo test` for the new `truncate_chars` test and the two regression tests; confirm they pass.
- [x] 4.2 Grep `src/web/portal/admin/` for `[..` byte-slice patterns on free-text values and confirm only the known-safe sites remain (`audit.rs` `short_id` guarded by `len() > 8`; `billing.rs:343` on a UUID string).
