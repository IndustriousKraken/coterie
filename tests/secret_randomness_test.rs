//! Guards for the one property this codebase cannot afford to lose
//! quietly: every value generated as a secret comes from a
//! cryptographically secure, OS-seeded generator, at unchanged length.
//!
//! A substitution — `SmallRng`, `StdRng::seed_from_u64`, anything a
//! caller could reproduce — compiles, produces output of exactly the
//! right shape, and passes every format test in the suite while making
//! the values predictable. There is no natural signal for that, so
//! these tests are the signal: behavioural checks where the site is
//! reachable, and a source scan where it isn't (a CSP nonce generated
//! inline in middleware has no function to call).

use std::fs;
use std::path::PathBuf;

use coterie::auth::{csrf::CsrfService, tokens::generate_token};

/// Enough draws that a 32-bit-state generator would be overwhelmingly
/// likely to collide, cheap enough to stay in the default test run.
const DRAWS: usize = 1000;

/// Every file in the tree that draws bytes for a secret, plus
/// `src/bin/seed.rs` — it is a seeding tool, not a reason to seed the
/// generator, and a reviewer skimming a `seed` file using a seeded RNG
/// reads it as normal.
const SECRET_SITES: &[&str] = &[
    "src/auth/tokens.rs",
    "src/auth/totp.rs",
    "src/auth/recovery_codes.rs",
    "src/auth/csrf.rs",
    "src/api/middleware/security_headers.rs",
    "src/service/member_service/bulk_import.rs",
    "src/bin/seed.rs",
    "deploy/coterie-provision/src/install/inputs.rs",
];

/// Seedable, reproducible, or small-state generators, plus rand 0.8's
/// `thread_rng` — the name this project migrated off, kept here so a
/// half-reverted merge is caught rather than silently resurrecting the
/// old API alongside the new one.
const FORBIDDEN: &[&str] = &[
    "SmallRng",
    "StdRng",
    "SeedableRng",
    "seed_from_u64",
    "from_seed",
    "StepRng",
    "ChaCha8Rng",
    "ChaCha12Rng",
    "ChaCha20Rng",
    "thread_rng",
];

/// The declared output length at each site. Changing any of these
/// changes a secret's keyspace, which is never an incidental edit.
const LENGTHS: &[(&str, &str)] = &[
    ("src/auth/tokens.rs", "[0u8; 32]"),
    ("src/auth/totp.rs", "const SECRET_LEN: usize = 20;"),
    ("src/auth/recovery_codes.rs", "const GROUP_LEN: usize = 4;"),
    ("src/auth/recovery_codes.rs", "const GROUPS: usize = 3;"),
    ("src/auth/csrf.rs", "const NONCE_LEN: usize = 16;"),
    ("src/api/middleware/security_headers.rs", "[0u8; 24]"),
    ("src/service/member_service/bulk_import.rs", "[0u8; 24]"),
    (
        "deploy/coterie-provision/src/install/inputs.rs",
        "[0u8; 32]",
    ),
];

fn read_site(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn tokens_are_32_bytes_hex_and_never_repeat() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..DRAWS {
        let token = generate_token();
        assert_eq!(token.len(), 64, "32 bytes hex-encoded is 64 chars");
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(seen.insert(token), "token repeated within {DRAWS} draws");
    }
}

#[tokio::test]
async fn csrf_nonces_never_repeat_for_the_same_session() {
    // Same key, same session id, so the MAC half is fixed: any two
    // tokens differing means the nonce differed.
    let svc = CsrfService::new("test-session-secret");
    let mut seen = std::collections::HashSet::new();
    for _ in 0..DRAWS {
        let token = svc.generate_token("session-abc").await.expect("generate");
        assert_eq!(token.len(), 96, "16-byte nonce + 32-byte MAC, hex-encoded");
        assert!(seen.insert(token), "nonce repeated within {DRAWS} draws");
    }
}

#[test]
fn no_seedable_or_small_state_generator_at_a_secret_site() {
    for site in SECRET_SITES {
        let src = read_site(site);
        for needle in FORBIDDEN {
            assert!(
                !src.contains(needle),
                "{site} references `{needle}`: secrets must come from the \
                 OS-seeded thread-local CSPRNG (`rand::rng()`), which no \
                 caller can reproduce"
            );
        }
        assert!(
            src.contains("rand::rng()"),
            "{site} no longer draws from `rand::rng()` — if the call was \
             renamed upstream again, update this assertion along with the \
             call sites; if the site stopped generating secrets, drop it \
             from SECRET_SITES"
        );
    }
}

#[test]
fn secret_lengths_are_unchanged() {
    for (site, decl) in LENGTHS {
        let src = read_site(site);
        assert!(
            src.contains(decl),
            "{site} no longer declares `{decl}` — this migration changes no \
             output length, encoding, or format"
        );
    }
}
