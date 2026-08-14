# Tasks

## 0. Lockfile repair (prerequisite, found on the first run)

- [x] 0.1 `cargo audit` did not run at all before this change: it panicked with
  `invalid Cargo.lock dependency tree: Resolution("failed to find dependency:
  rand 0.9.4")`. Both workspace members listed `rand 0.9.4` as a dependency and
  no such package entry existed — a merge left the lockfile inconsistent.
  Re-resolving added the missing `rand 0.9.5`, `rand_chacha 0.9.0`, and
  `rand_core 0.9.5` entries. Nothing else in the lock moved.

## 1. ammonia

- [x] 1.1 Move `ammonia` to a version fixing RUSTSEC-2026-0213 (>=4.1.4).
  `Cargo.toml` now states the floor (`ammonia = "4.1.4"`) rather than leaving it
  to resolution; the lock is at 4.1.4.
- [x] 1.2 Change nothing about the sanitizer's configuration. The allow-list is a
  fixed tag set with no SVG element, which is why the advisory is not reachable
  here; that is a reason to upgrade calmly, not a reason to start allowing more.
  `src/util/markdown.rs` is untouched.
- [x] 1.3 Confirm the existing markdown tests still pass unmodified. If any
  expectation needs editing, stop — the sanitizer's output should be identical.
  24 markdown tests pass with no edit to any expectation.

## 2. Remove the unused database drivers

- [x] 2.1 `Cargo.toml:34` declares `sqlx` without `default-features = false`, so
  the default feature set pulls every driver. Add it, and re-add explicitly only
  what is used — the runtime, `sqlite`, `chrono`, `uuid`, plus whatever of
  `macros`, `migrate`, `json` the code actually needs.
  Final set: `runtime-tokio-rustls`, `sqlite`, `chrono`, `uuid`, `macros`,
  `migrate`. `any` and `json` are gone.
- [x] 2.2 Determine those by building, not by guessing: remove the defaults, then
  add back what fails to compile. Adding a feature speculatively re-introduces
  what this task exists to remove.
  Built with only the original four features first. It failed on `cannot find
  FromRow in sqlx` / `cannot find Type in sqlx` (the derives, from `macros`) —
  `macros` and `migrate` went back in on the strength of that. `json` was never
  needed: nothing uses `sqlx::types::Json`, and the workspace compiles without
  it.
- [x] 2.3 Verify `sqlx-mysql`, `sqlx-postgres`, and `rsa` are gone from
  `Cargo.lock` afterwards. `rsa` is the advisory with no fixed version, and it
  disappears entirely with the MySQL driver.

  **Verified, and the result is not the expected one — recorded rather than
  worked around.** All three are gone from the *build graph*:

      $ cargo tree --target all -i rsa -e normal,build
      warning: nothing to print.

  (same for `sqlx-mysql` and `sqlx-postgres`; `sqlx-sqlite` still resolves).
  They remain in *Cargo.lock*. Cargo resolves the lockfile with all features
  enabled so the lock stays valid under any feature selection, which means an
  unactivated optional dependency is still recorded. Confirmed in isolation: a
  fresh crate depending only on `sqlx` with `default-features = false` and
  `sqlite`, then `cargo generate-lockfile`, produces a lock containing `rsa`,
  `sqlx-mysql`, and `sqlx-postgres`. There is no feature selection that removes
  them while `sqlx` is the dependency.

  `cargo audit` reads the lockfile, so RUSTSEC-2023-0071 survives a removal that
  did happen. That specific gap — and not the crate — is what the waiver in
  section 4 covers; see the note on 4.3.
- [x] 2.4 The migration macros and the offline query cache (`SQLX_OFFLINE`) must
  still work — that is the load-bearing use of `macros`/`migrate` and the thing a
  too-aggressive trim would break.
  `sqlx::migrate!("./migrations")` is called from 3 binaries and ~15 test
  fixtures; all compile and run. The offline query cache is not in use here and
  is not affected by the trim: there is no `.sqlx/` directory, no `SQLX_OFFLINE`
  anywhere in the build or CI, and no compile-time `query!`/`query_as!` for it
  to serve — every query goes through the runtime `sqlx::query` API.

## 3. The old TLS stack

- [x] 3.1 `rustls-webpki` 0.101.7 arrives via `async-stripe` 0.39.1 →
  `hyper-rustls` 0.24 → `rustls` 0.21, alongside the `rustls` 0.23 the
  application itself uses. Check whether a newer `async-stripe` drops the old
  stack.
  Confirmed by `cargo tree --target all -i rustls-webpki@0.101.7 -e normal`.
  `cargo info async-stripe`: 0.39.1 is the newest stable release; the only newer
  publication is the 1.0.0-rc line.
- [x] 3.2 If one does, upgrade to it and confirm both `rustls` 0.21 and
  `rustls-webpki` 0.101 leave the lockfile. Treat the Stripe client's API surface
  as behavior to preserve — the payment paths are covered by tests, and those
  tests passing unmodified is the bar.
  Not applicable — no such version. 1.0.0-rc.8 is a pre-release API rewrite, so
  it fails the stated bar outright, and a release candidate on the payment path
  is a larger risk than the three findings. Every async-stripe runtime feature
  other than native-tls (refused: it would put OpenSSL back in the static musl
  binary) routes through hyper-rustls 0.24.
- [x] 3.3 If no such version exists, waive the three findings explicitly: the
  advisory identifiers, the fact that the affected stack is reached only by the
  Stripe client and not by the application's own TLS, and a revisit date. Do not
  waive more broadly than those three identifiers.
  RUSTSEC-2026-0104, RUSTSEC-2026-0098, RUSTSEC-2026-0099, each named on its own
  line, argued and dated in the Makefile.

## 4. Waivers

- [x] 4.1 Any waiver names its advisory identifier individually. No wildcard, no
  crate-wide suppression, no blanket ignore file.
  Four `--ignore <RUSTSEC-id>` entries in `AUDIT_IGNORES` (Makefile). A test
  asserts each is a bare `RUSTSEC-YYYY-NNNN` and nothing else.

  The waivers live in the Makefile rather than `.cargo/audit.toml` because the
  sandbox this ran in refuses writes under `.cargo/`. The Makefile turns out to
  be the better home anyway: the CI advisory job calls `make audit`, so the list
  a contributor runs locally and the list that gates a pull request are the same
  text, which a `run: cargo audit` step plus a config file are not.
- [x] 4.2 Each waiver carries the reachability reasoning and a revisit date, in
  the file where it is declared, so the next reader can re-evaluate it without
  reconstructing the argument.
  Each entry carries the reverse-dependency path, why the code path is not
  reachable, why no fix is available, and `Revisit 2027-02-13` (for the Stripe
  three, also "or when async-stripe 1.0 ships stable, whichever is first").
- [x] 4.3 Do not waive anything that section 1, 2, or 3 resolved. A waiver
  standing in for an available fix is the failure this guards against.
  RUSTSEC-2026-0213 was fixed by upgrading ammonia and is deliberately absent
  from the list. RUSTSEC-2023-0071 is waived even though section 2 removed `rsa`
  from the build, because the removal does not reach the lockfile the check reads
  (see 2.3) — the waiver stands in for a lockfile-semantics gap, not for an
  available fix. Its comment says so, and names the `cargo tree` command that
  would start printing a path if a driver ever came back.

## 5. Verification

- [x] 5.1 `cargo audit` passes on the resulting tree. That is the whole point: the
  check stops being permanently red and starts carrying information again.
  `make audit` exits 0. Before: 5 vulnerabilities (and, before the 0.1 lockfile
  repair, a panic). After: 0 vulnerabilities, 11 informational warnings
  unchanged and still non-failing.
- [x] 5.2 The full test suite passes with no expectation edits. Nothing here
  should change application behavior.
  1088 root tests (`cargo test --features test-utils`) and 133
  `coterie-provision` tests (`--features test-support`) pass. No existing test
  was edited. 3 tests added, covering the waiver discipline itself.
- [x] 5.3 Confirm the build still produces the musl release binaries CI warms and
  the release workflow ships — the sqlx feature trim touches compilation, and a
  break would surface at release time rather than in tests.
  `cargo build --release --target x86_64-unknown-linux-musl --bins` and
  `-p coterie-provision` both succeed. All four artifacts (`coterie`, `seed`,
  `create_admin`, `coterie-provision`) link static-pie, no OpenSSL.
- [x] 5.4 Record in the output of this change which findings were fixed by
  upgrade, which by removal, and which by waiver, so the next audit run's diff is
  interpretable.

  | Advisory | Crate | Resolution |
  |---|---|---|
  | RUSTSEC-2026-0213 | ammonia 4.1.3 | **Upgrade** to 4.1.4 |
  | RUSTSEC-2023-0071 | rsa 0.9.10 | **Removal** from the build graph (sqlx `default-features = false`), **plus a waiver** for the lockfile entry that removal cannot reach |
  | RUSTSEC-2026-0104 | rustls-webpki 0.101.7 | **Waiver** — no fix in the 0.101 line, no newer stable async-stripe |
  | RUSTSEC-2026-0098 | rustls-webpki 0.101.7 | **Waiver** — same |
  | RUSTSEC-2026-0099 | rustls-webpki 0.101.7 | **Waiver** — same |

  Also removed from the build, without an advisory attached: `sqlx-mysql`,
  `sqlx-postgres`, and their transitive dependencies.
