# Tasks

## 1. The bump

- [x] 1.1 `Cargo.toml`: `rand` from `"0.8"` to `"0.9"`. Do not adopt Dependabot
  PR #81 as-is — it is conflicted and the manifest line is the smallest part of
  the work.
- [x] 1.2 Regenerate the lockfile. The tree currently resolves `rand` 0.8.6 and
  0.9.4 side by side; after this it should not need both for direct use.

## 2. Call sites

- [x] 2.1 Migrate all ten call sites across the eight files that reference
  `rand::`. `thread_rng()` becomes the 0.9 equivalent; the `seq` trait imports in
  `src/auth/recovery_codes.rs` and `src/bin/seed.rs` move to their 0.9 paths.
- [x] 2.2 Keep the OS-seeded thread-local CSPRNG at every site. Do **not**
  substitute `SmallRng`, `StdRng::seed_from_u64`, or any seedable generator,
  including in `src/bin/seed.rs` — it is a seeding tool, not a reason to seed the
  generator, and a reviewer skimming the diff will read a `seed` file using a
  seeded RNG as normal.
- [x] 2.3 Change no output length, encoding, or format. This migration's success
  condition is that nothing observable changes.
- [x] 2.4 `deploy/coterie-provision` has its own manifest — migrate it in the same
  change so the two do not diverge on a security-relevant dependency.

## 3. Guards

- [x] 3.1 Add a test asserting that generated secrets do not repeat across many
  generations and are not reproducible from a caller-available seed. This is the
  signal the change otherwise lacks: a substitution compiles and produces
  correctly-shaped output.
- [x] 3.2 Add a grep-style assertion that no seedable or small-state generator
  appears at a secret-generating site. The behavior-preserving nature of this
  change means a wrong version passes every functional test.
- [x] 3.3 Assert output lengths are unchanged at each site — 32 bytes for tokens,
  and the existing lengths for TOTP secrets, recovery codes, nonces, and
  generated passwords.

## 4. Verification

- [x] 4.1 Full test suite passes with no expectation edits. If a test needs
  changing to accommodate this, stop and report it — a behavior-preserving
  migration that requires editing assertions is not behavior-preserving.
- [x] 4.2 Confirm no remaining reference to the 0.8 API anywhere, including the
  provisioning crate and any documentation or comments that name it.
- [x] 4.3 Confirm canon no longer names a specific function for randomness, so
  the next upstream rename is a code change and not a spec violation.
