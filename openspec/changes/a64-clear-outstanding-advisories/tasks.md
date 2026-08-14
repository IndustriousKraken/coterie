# Tasks

## 1. ammonia

- [ ] 1.1 Move `ammonia` to a version fixing RUSTSEC-2026-0213 (>=4.1.4).
- [ ] 1.2 Change nothing about the sanitizer's configuration. The allow-list is a
  fixed tag set with no SVG element, which is why the advisory is not reachable
  here; that is a reason to upgrade calmly, not a reason to start allowing more.
- [ ] 1.3 Confirm the existing markdown tests still pass unmodified. If any
  expectation needs editing, stop — the sanitizer's output should be identical.

## 2. Remove the unused database drivers

- [ ] 2.1 `Cargo.toml:34` declares `sqlx` without `default-features = false`, so
  the default feature set pulls every driver. Add it, and re-add explicitly only
  what is used — the runtime, `sqlite`, `chrono`, `uuid`, plus whatever of
  `macros`, `migrate`, `json` the code actually needs.
- [ ] 2.2 Determine those by building, not by guessing: remove the defaults, then
  add back what fails to compile. Adding a feature speculatively re-introduces
  what this task exists to remove.
- [ ] 2.3 Verify `sqlx-mysql`, `sqlx-postgres`, and `rsa` are gone from
  `Cargo.lock` afterwards. `rsa` is the advisory with no fixed version, and it
  disappears entirely with the MySQL driver.
- [ ] 2.4 The migration macros and the offline query cache (`SQLX_OFFLINE`) must
  still work — that is the load-bearing use of `macros`/`migrate` and the thing a
  too-aggressive trim would break.

## 3. The old TLS stack

- [ ] 3.1 `rustls-webpki` 0.101.7 arrives via `async-stripe` 0.39.1 →
  `hyper-rustls` 0.24 → `rustls` 0.21, alongside the `rustls` 0.23 the
  application itself uses. Check whether a newer `async-stripe` drops the old
  stack.
- [ ] 3.2 If one does, upgrade to it and confirm both `rustls` 0.21 and
  `rustls-webpki` 0.101 leave the lockfile. Treat the Stripe client's API surface
  as behavior to preserve — the payment paths are covered by tests, and those
  tests passing unmodified is the bar.
- [ ] 3.3 If no such version exists, waive the three findings explicitly: the
  advisory identifiers, the fact that the affected stack is reached only by the
  Stripe client and not by the application's own TLS, and a revisit date. Do not
  waive more broadly than those three identifiers.

## 4. Waivers

- [ ] 4.1 Any waiver names its advisory identifier individually. No wildcard, no
  crate-wide suppression, no blanket ignore file.
- [ ] 4.2 Each waiver carries the reachability reasoning and a revisit date, in
  the file where it is declared, so the next reader can re-evaluate it without
  reconstructing the argument.
- [ ] 4.3 Do not waive anything that section 1, 2, or 3 resolved. A waiver
  standing in for an available fix is the failure this guards against.

## 5. Verification

- [ ] 5.1 `cargo audit` passes on the resulting tree. That is the whole point: the
  check stops being permanently red and starts carrying information again.
- [ ] 5.2 The full test suite passes with no expectation edits. Nothing here
  should change application behavior.
- [ ] 5.3 Confirm the build still produces the musl release binaries CI warms and
  the release workflow ships — the sqlx feature trim touches compilation, and a
  break would surface at release time rather than in tests.
- [ ] 5.4 Record in the output of this change which findings were fixed by
  upgrade, which by removal, and which by waiver, so the next audit run's diff is
  interpretable.
