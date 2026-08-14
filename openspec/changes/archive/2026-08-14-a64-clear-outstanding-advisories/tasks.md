# Tasks

## 1. ammonia

- [x] 1.1 Move `ammonia` to a version fixing RUSTSEC-2026-0213 (>=4.1.4).
- [x] 1.2 Change nothing about the sanitizer's configuration. The allow-list is a
  fixed tag set with no SVG element, which is why the advisory is not reachable
  here; that is a reason to upgrade calmly, not a reason to start allowing more.
- [x] 1.3 Confirm the existing markdown tests still pass unmodified. If any
  expectation needs editing, stop — the sanitizer's output should be identical.

## 2. Remove the unused database drivers

- [x] 2.1 `Cargo.toml:34` declares `sqlx` without `default-features = false`, so
  the default feature set pulls every driver. Add it, and re-add explicitly only
  what is used — the runtime, `sqlite`, `chrono`, `uuid`, plus whatever of
  `macros`, `migrate`, `json` the code actually needs.
- [x] 2.2 Determine those by building, not by guessing: remove the defaults, then
  add back what fails to compile. Adding a feature speculatively re-introduces
  what this task exists to remove.
- [x] 2.3 Verify `sqlx-mysql`, `sqlx-postgres`, and `rsa` are gone from
  `Cargo.lock` afterwards. `rsa` is the advisory with no fixed version, and it
  disappears entirely with the MySQL driver.

  **Verified, and the premise does not hold.** The three crates were already
  absent from the *build* graph before this change — sqlx's defaults are
  `any`/`macros`/`migrate`/`json`, and each reaches the drivers through a weak
  `sqlx-mysql?/...` reference that activates nothing, so `cargo tree -i rsa
  --target all` printed nothing even on the pre-change tree. They stay in
  `Cargo.lock` because Cargo records a package's optional dependencies whether or
  not a feature enables them. Confirmed against a scratch crate whose only
  dependency was `sqlx` with `default-features = false` and the sqlite feature
  set: its freshly resolved lockfile still contained `sqlx-mysql`, `sqlx-postgres`
  and `rsa`. No feature declaration can remove them; only sqlx dropping the
  optional dependency would. RUSTSEC-2023-0071 is therefore waived under
  section 4 with that reasoning recorded, not left unaddressed. The feature trim
  is kept — it drops `any` and `json` from the compile and states the SQLite-only
  intent at the declaration.
- [x] 2.4 The migration macros and the offline query cache (`SQLX_OFFLINE`) must
  still work — that is the load-bearing use of `macros`/`migrate` and the thing a
  too-aggressive trim would break. `macros` + `migrate` are retained; the 20-odd
  `sqlx::migrate!("./migrations")` call sites compile and every migration-running
  test passes. There is no offline query cache to preserve — the project has no
  `.sqlx/` directory and no compile-time `query!`/`query_as!` macros, so nothing
  consumes `SQLX_OFFLINE`.

## 3. The old TLS stack

- [x] 3.1 `rustls-webpki` 0.101.7 arrives via `async-stripe` 0.39.1 →
  `hyper-rustls` 0.24 → `rustls` 0.21, alongside the `rustls` 0.23 the
  application itself uses. Check whether a newer `async-stripe` drops the old
  stack. Path confirmed exactly as described by `cargo tree -i
  rustls-webpki@0.101.7`.
- [x] 3.2 If one does, upgrade to it and confirm both `rustls` 0.21 and
  `rustls-webpki` 0.101 leave the lockfile. Treat the Stripe client's API surface
  as behavior to preserve — the payment paths are covered by tests, and those
  tests passing unmodified is the bar. **No such version.** 0.39.1 is the newest
  stable `async-stripe` (checked 2026-08-14); the only newer publication is
  1.0.0-rc.8, a prerelease that reorganises the crate into a different API
  surface. It fails the bar this task sets — the payment paths could not survive
  it unmodified — so 3.3 applies.
- [x] 3.3 If no such version exists, waive the three findings explicitly: the
  advisory identifiers, the fact that the affected stack is reached only by the
  Stripe client and not by the application's own TLS, and a revisit date. Do not
  waive more broadly than those three identifiers.

## 4. Waivers

- [x] 4.1 Any waiver names its advisory identifier individually. No wildcard, no
  crate-wide suppression, no blanket ignore file. Four `--ignore <ID>` flags on
  the `cargo audit` invocation in `.github/workflows/ci.yml`, one identifier each.
  (The idiomatic home for these would be `.cargo/audit.toml`; that path is not
  writable in this sandbox, so the waivers live at the invocation the check
  actually runs, which is also where a reader meets them.)
- [x] 4.2 Each waiver carries the reachability reasoning and a revisit date, in
  the file where it is declared, so the next reader can re-evaluate it without
  reconstructing the argument. Revisit 2026-11-14 for all four, plus the earlier
  trigger conditions: a MySQL driver being enabled (rsa), or `async-stripe` 1.0
  shipping stable (the three rustls-webpki findings).
- [x] 4.3 Do not waive anything that section 1, 2, or 3 resolved. A waiver
  standing in for an available fix is the failure this guards against.
  RUSTSEC-2026-0213 is fixed by upgrade and is not in the ignore list.

## 5. Verification

- [x] 5.1 `cargo audit` passes on the resulting tree. That is the whole point: the
  check stops being permanently red and starts carrying information again. Exit 0,
  0 vulnerabilities, 11 informational warnings (unchanged, non-failing).

  Note: `cargo audit` could not run at all on the pre-change tree — `Cargo.lock`
  referenced `rand 0.9.4` (since yanked) with no matching package entry, and the
  audit panicked with `invalid Cargo.lock dependency tree`. Re-resolving to
  `rand 0.9.5` was a prerequisite, not a version bump for its own sake.
- [x] 5.2 The full test suite passes with no expectation edits. Nothing here
  should change application behavior. 1084 passed / 0 failed / 3 ignored
  (`cargo test --features test-utils`), plus 132 passed / 0 failed for
  `coterie-provision --features test-support`. No test file was touched.
- [x] 5.3 Confirm the build still produces the musl release binaries CI warms and
  the release workflow ships — the sqlx feature trim touches compilation, and a
  break would surface at release time rather than in tests. Both
  `cargo build --release --target x86_64-unknown-linux-musl --bins` and the
  `-p coterie-provision` build succeed; `coterie` links static-pie as before.
- [x] 5.4 Record in the output of this change which findings were fixed by
  upgrade, which by removal, and which by waiver, so the next audit run's diff is
  interpretable.

  | Advisory | Crate | Disposition |
  |---|---|---|
  | RUSTSEC-2026-0213 | ammonia 4.1.3 → 4.1.4 | Fixed by upgrade |
  | RUSTSEC-2023-0071 | rsa 0.9.10 | Waived — not compiled; unremovable from the lockfile (see 2.3) |
  | RUSTSEC-2026-0104 | rustls-webpki 0.101.7 | Waived — Stripe client's TLS only, no fixed version reachable |
  | RUSTSEC-2026-0098 | rustls-webpki 0.101.7 | Waived — same |
  | RUSTSEC-2026-0099 | rustls-webpki 0.101.7 | Waived — same |

  Nothing was fixed by removal: the removal section 2 aimed at is not achievable
  through Cargo features, for the reason recorded under 2.3. The feature trim it
  produced is kept anyway, and `any`/`json` did leave the compile.
