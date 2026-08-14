//! The advisory waivers' discipline, checked statically.
//!
//! A waiver is the one way a finding leaves the advisory check without being
//! fixed, so the shape of the list is the whole safeguard: each entry names one
//! RustSec identifier, and each identifier is argued for beside where it is
//! declared with a date it gets re-read. Nothing enforces that — `cargo audit`
//! accepts whatever it is handed, and a crate-wide or wildcard suppression
//! would silence advisories nobody has looked at yet, including ones published
//! after the waiver was written.
//!
//! The list lives in the Makefile's `AUDIT_IGNORES`, which the CI advisory job
//! reaches through `make audit`. That indirection is asserted too: a workflow
//! that went back to calling `cargo audit` directly would either fail on the
//! waived findings or grow a second copy of the list to drift against.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn makefile() -> String {
    fs::read_to_string(repo_root().join("Makefile")).expect("read Makefile")
}

/// Every `--ignore <id>` argument in `AUDIT_IGNORES`, in declaration order.
fn waived_advisories() -> Vec<String> {
    makefile()
        .split("--ignore")
        .skip(1)
        .map(|rest| {
            rest.split_whitespace()
                .next()
                .expect("--ignore is followed by an argument")
                .to_string()
        })
        .collect()
}

/// A RustSec identifier and nothing else: `RUSTSEC-<year>-<number>`. A crate
/// name, a glob, or a trailing wildcard all fail here.
fn is_advisory_id(s: &str) -> bool {
    let mut parts = s.split('-');
    parts.next() == Some("RUSTSEC")
        && parts
            .next()
            .is_some_and(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()))
        && parts
            .next()
            .is_some_and(|n| n.len() == 4 && n.chars().all(|c| c.is_ascii_digit()))
        && parts.next().is_none()
}

#[test]
fn every_waiver_names_a_single_advisory() {
    let waived = waived_advisories();
    assert!(
        !waived.is_empty(),
        "AUDIT_IGNORES parsed as empty — the parser or the Makefile moved"
    );
    for id in &waived {
        assert!(
            is_advisory_id(id),
            "waived {id:?} is not a bare RustSec identifier; a crate-wide or \
             wildcard suppression hides advisories nobody has read"
        );
    }
}

#[test]
fn every_waiver_is_argued_and_dated() {
    let makefile = makefile();
    for id in waived_advisories() {
        // The `--ignore` line itself is one mention; the comment block above
        // it is the other. One mention means an entry nobody explained.
        let mentions = makefile.matches(&id).count();
        assert!(
            mentions >= 2,
            "{id} appears only in the ignore list — a waiver carries its \
             reachability reasoning beside it"
        );
    }

    let revisits = makefile.matches("Revisit ").count();
    assert!(
        revisits > 0,
        "no waiver states a revisit date; an undated waiver is permanent by \
         default"
    );
}

#[test]
fn ci_runs_the_audit_through_the_makefile() {
    let ci = fs::read_to_string(repo_root().join(".github/workflows/ci.yml")).expect("read ci.yml");

    assert!(
        ci.contains("run: make audit"),
        "the advisory job no longer runs `make audit`; the waiver list and the \
         check that reads it have to stay in one place"
    );
    // A direct `cargo audit ...` step would be a second, silently diverging
    // copy of the list. Comments explaining the indirection are fine.
    for line in ci.lines() {
        let step = line.trim_start();
        assert!(
            !step.starts_with("run: cargo audit"),
            "ci.yml invokes cargo audit directly: {step}"
        );
    }
}
