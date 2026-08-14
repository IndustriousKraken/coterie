//! The workflow supply chain: least-privilege tokens, verified action pins, and
//! a deploy path with no third party in it.
//!
//! Two kinds of assertion live here. The pin checker
//! (`scripts/verify_action_pins.py`) is driven against fixture workflows with a
//! canned transport, so its logic — including the annotated-tag dereference that
//! a first implementation gets wrong — is exercised offline; CI runs the same
//! code against the live API. Everything else is a property of the workflow
//! files themselves, which is where the property actually lives: a run cannot
//! reach the deploy host, so "the key is not handed to a third party" is read
//! off the file, not observed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workflows_dir() -> PathBuf {
    repo_root().join(".github/workflows")
}

fn checker() -> PathBuf {
    repo_root().join("scripts/verify_action_pins.py")
}

/// Synthetic SHAs. The fixture cases assert the checker's reasoning, not this
/// repository's pins, so they use commits that exist nowhere.
const SHA_A: &str = "1111111111111111111111111111111111111111";
const SHA_B: &str = "2222222222222222222222222222222222222222";
const TAG_OBJECT: &str = "3333333333333333333333333333333333333333";

/// A throwaway workflow directory plus its canned API responses.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("coterie-pin-check-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("workflows")).expect("scratch dir is creatable");
        Scratch(dir)
    }

    fn workflow(self, name: &str, body: &str) -> Self {
        fs::write(self.0.join("workflows").join(name), body).expect("workflow is writable");
        self
    }

    /// Runs the checker over this directory. Returns (passed, stderr).
    fn check(&self, refs: &str) -> (bool, String) {
        let refs_path = self.0.join("refs.json");
        fs::write(&refs_path, refs).expect("fixture is writable");
        run_checker(&self.0.join("workflows"), &refs_path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_checker(workflows: &Path, refs: &Path) -> (bool, String) {
    let output = Command::new("python3")
        .arg(checker())
        .arg("--workflows")
        .arg(workflows)
        .arg("--refs")
        .arg(refs)
        .output()
        .expect("python3 is needed to run scripts/verify_action_pins.py");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A tag that resolves straight to a commit (a lightweight tag).
fn lightweight(action: &str, tag: &str, commit: &str) -> String {
    format!(
        "\"/repos/{action}/git/ref/tags/{tag}\": \
         {{\"object\": {{\"type\": \"commit\", \"sha\": \"{commit}\"}}}}"
    )
}

/// A tag whose ref points at a tag object, which in turn names the commit.
fn annotated(action: &str, tag: &str, tag_object: &str, commit: &str) -> String {
    format!(
        "\"/repos/{action}/git/ref/tags/{tag}\": \
         {{\"object\": {{\"type\": \"tag\", \"sha\": \"{tag_object}\"}}}}, \
         \"/repos/{action}/git/tags/{tag_object}\": \
         {{\"object\": {{\"sha\": \"{commit}\"}}}}"
    )
}

fn commit_exists(action: &str, commit: &str) -> String {
    format!("\"/repos/{action}/git/commits/{commit}\": {{\"sha\": \"{commit}\"}}")
}

fn step(uses: &str) -> String {
    format!("jobs:\n  j:\n    steps:\n      - uses: {uses}\n")
}

// ---------------------------------------------------------------------------
// Pin verification (tasks 4.1 - 4.8)
// ---------------------------------------------------------------------------

#[test]
fn a_tag_reference_is_rejected() {
    let scratch = Scratch::new("tag-ref").workflow("w.yml", &step("acme/action@v1.2.3 # v1.2.3"));
    let (passed, stderr) = scratch.check("{}");

    assert!(!passed, "a tag reference must fail:\n{stderr}");
    assert!(
        stderr.contains("acme/action@v1.2.3") && stderr.contains("full 40-character"),
        "the failure must name the offending reference:\n{stderr}"
    );
}

#[test]
fn an_annotated_tag_is_dereferenced_before_comparison() {
    // `git/ref/tags/v1.2.3` yields the TAG OBJECT here, not the commit. This is
    // the shape `Swatinem/rust-cache` v2.9.2 actually has; comparing without
    // following it reports a correct pin as a mismatch, and a check that cries
    // wolf gets muted.
    let refs = format!(
        "{{{}}}",
        annotated("acme/action", "v1.2.3", TAG_OBJECT, SHA_A)
    );

    let correct = Scratch::new("annotated-ok")
        .workflow("w.yml", &step(&format!("acme/action@{SHA_A} # v1.2.3")));
    let (passed, stderr) = correct.check(&refs);
    assert!(passed, "a correct annotated-tag pin must pass:\n{stderr}");

    // And the dereference is real rather than skipped: pinning the tag object
    // itself — what an undereferenced comparison would accept — must fail.
    let tag_object_pin = Scratch::new("annotated-tagobj").workflow(
        "w.yml",
        &step(&format!("acme/action@{TAG_OBJECT} # v1.2.3")),
    );
    let (passed, stderr) = tag_object_pin.check(&refs);
    assert!(
        !passed && stderr.contains(SHA_A),
        "pinning the tag object must fail and name the real commit:\n{stderr}"
    );
}

#[test]
fn a_sha_that_disagrees_with_its_comment_fails() {
    let scratch =
        Scratch::new("mismatch").workflow("w.yml", &step(&format!("acme/action@{SHA_A} # v1.2.3")));
    let (passed, stderr) = scratch.check(&format!(
        "{{{}}}",
        lightweight("acme/action", "v1.2.3", SHA_B)
    ));

    assert!(!passed, "a mismatched pin must fail:\n{stderr}");
    assert!(
        stderr.contains(SHA_A) && stderr.contains(SHA_B),
        "the failure must name both the pin and what upstream says:\n{stderr}"
    );
}

#[test]
fn a_pin_with_no_version_comment_fails() {
    let scratch =
        Scratch::new("no-comment").workflow("w.yml", &step(&format!("acme/action@{SHA_A}")));
    let (passed, stderr) = scratch.check("{}");

    assert!(
        !passed,
        "an unverifiable pin is not a verified one:\n{stderr}"
    );
    assert!(
        stderr.contains("no version comment"),
        "the failure must say what is missing:\n{stderr}"
    );
}

#[test]
fn a_version_absent_upstream_fails() {
    let scratch = Scratch::new("absent-tag")
        .workflow("w.yml", &step(&format!("acme/action@{SHA_A} # v9.9.9")));
    let (passed, stderr) = scratch.check("{}");

    assert!(!passed, "a nonexistent version must fail:\n{stderr}");
    assert!(
        stderr.contains("does not exist"),
        "the failure must say the version is not upstream:\n{stderr}"
    );
}

#[test]
fn one_action_at_two_shas_across_workflows_fails() {
    // Both pins match their own comment, so a per-pin check passes them both —
    // this is the `actions/checkout` v7.0.1/v7.0.0 split, where one job quietly
    // ran an older action than the repository believed it had standardized on.
    let refs = format!(
        "{{{}, {}}}",
        lightweight("acme/action", "v1.2.3", SHA_A),
        lightweight("acme/action", "v1.2.4", SHA_B)
    );

    let split = Scratch::new("split")
        .workflow("a.yml", &step(&format!("acme/action@{SHA_A} # v1.2.3")))
        .workflow("b.yml", &step(&format!("acme/action@{SHA_B} # v1.2.4")));
    let (passed, stderr) = split.check(&refs);
    assert!(!passed, "a split pin must fail:\n{stderr}");
    assert!(
        stderr.contains(SHA_A) && stderr.contains(SHA_B),
        "the failure must name both references:\n{stderr}"
    );

    let agreed = Scratch::new("agreed")
        .workflow("a.yml", &step(&format!("acme/action@{SHA_A} # v1.2.3")))
        .workflow("b.yml", &step(&format!("acme/action@{SHA_A} # v1.2.3")));
    let (passed, stderr) = agreed.check(&refs);
    assert!(passed, "agreeing occurrences must pass:\n{stderr}");
}

#[test]
fn a_moving_reference_verifies_by_existence_not_equality() {
    // The live `dtolnay/rust-toolchain` case: `stable` has been force-pushed
    // past the pinned commit — it is neither the branch head nor an ancestor of
    // it — while the commit itself is still in the repository. The fixture
    // models exactly that by answering only the commit lookup.
    let scratch = Scratch::new("moving-ok").workflow(
        "w.yml",
        &step(&format!(
            "acme/action@{SHA_A} # stable branch — moving ref, pinned 2026-07-09"
        )),
    );
    let (passed, stderr) = scratch.check(&format!("{{{}}}", commit_exists("acme/action", SHA_A)));

    assert!(
        passed,
        "a moving-reference pin whose commit exists must pass:\n{stderr}"
    );
}

#[test]
fn a_moving_reference_outside_the_repository_fails() {
    let scratch = Scratch::new("moving-absent").workflow(
        "w.yml",
        &step(&format!(
            "acme/action@{SHA_A} # stable branch — moving ref, pinned 2026-07-09"
        )),
    );
    // A commit from a fork, a typo, or an invention: nothing answers for it.
    let (passed, stderr) = scratch.check("{}");

    assert!(!passed, "a commit absent upstream must fail:\n{stderr}");
    assert!(
        stderr.contains(SHA_A) && stderr.contains("not in acme/action"),
        "the failure must name the offending reference:\n{stderr}"
    );
}

#[test]
fn a_tag_comment_mentioning_a_branch_is_still_a_tag_claim() {
    // The marker is positional — second token — so prose that happens to say
    // "branch" or "moving" does not quietly drop the pin to existence-only,
    // which is the whole assertion for ten of the fourteen pins here.
    let scratch = Scratch::new("incidental-branch").workflow(
        "w.yml",
        &step(&format!(
            "acme/action@{SHA_A} # v1.2.3 fixes branch handling"
        )),
    );
    let refs = format!(
        "{{{}, {}}}",
        lightweight("acme/action", "v1.2.3", SHA_B),
        commit_exists("acme/action", SHA_A)
    );
    let (passed, stderr) = scratch.check(&refs);

    assert!(
        !passed && stderr.contains("claims tag 'v1.2.3'"),
        "a comment that merely mentions a branch must still be held to \
         equality:\n{stderr}"
    );
}

#[test]
fn a_tag_claim_is_not_rescued_by_existence() {
    // Existence upstream is the fallback for moving references only. Lowering
    // every pin to it would discard the check's value for the ten of fourteen
    // pins here that name a tag.
    let scratch = Scratch::new("tag-not-rescued")
        .workflow("w.yml", &step(&format!("acme/action@{SHA_A} # v1.2.3")));
    let refs = format!(
        "{{{}, {}}}",
        lightweight("acme/action", "v1.2.3", SHA_B),
        commit_exists("acme/action", SHA_A)
    );
    let (passed, stderr) = scratch.check(&refs);

    assert!(
        !passed,
        "a tag claim must still be held to equality:\n{stderr}"
    );
}

#[test]
fn the_current_tree_verifies() {
    // Resolver-independent by construction: the fixture answers every claim
    // with the SHA the tree pins, so what remains under test is everything the
    // upstream API has no say in — that each reference is a full SHA, that each
    // names a version, and that no action appears at two commits. Whether the
    // tags themselves agree is CI's job, against the live API.
    let mut entries: Vec<String> = Vec::new();
    for used in workflow_uses() {
        let Some(claim) = used.claim.as_deref() else {
            continue; // the checker reports this; do not paper over it here
        };
        entries.push(if used.moving {
            commit_exists(&used.action, &used.sha)
        } else {
            lightweight(&used.action, claim, &used.sha)
        });
    }
    assert!(!entries.is_empty(), "the workflows must contain pins");

    let scratch = Scratch::new("current-tree");
    let refs_path = scratch.0.join("refs.json");
    fs::write(&refs_path, format!("{{{}}}", entries.join(", "))).expect("fixture is writable");
    let (passed, stderr) = run_checker(&workflows_dir(), &refs_path);

    assert!(passed, "the workflows as committed must verify:\n{stderr}");
}

// ---------------------------------------------------------------------------
// Least privilege (task 4.9)
// ---------------------------------------------------------------------------

#[test]
fn no_workflow_token_carries_write_scope_except_release() {
    for path in workflow_files() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let scopes = permission_scopes(&path);
        assert!(
            !scopes.is_empty(),
            "{name} declares no `permissions:` block, so its token is whatever \
             the repository default happens to be"
        );

        let writes: Vec<_> = scopes
            .iter()
            .filter(|(_, value)| value != "read" && value != "none")
            .collect();

        if name == "release.yml" {
            // The model the others were brought up to: one scope, the narrowest
            // that works, with a comment saying why.
            assert_eq!(
                writes.len(),
                1,
                "release.yml should need exactly one write scope, found {writes:?}"
            );
            assert_eq!(writes[0], &("contents".to_string(), "write".to_string()));
        } else {
            assert!(
                writes.is_empty(),
                "{name} grants {writes:?}; it reads the repository and nothing else"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The deploy credential path (task 4.10)
// ---------------------------------------------------------------------------

#[test]
fn deploy_hands_no_secret_to_a_third_party() {
    let deploy = workflows_dir().join("deploy.yml");
    let text = fs::read_to_string(&deploy).expect("deploy.yml is readable");

    for block in steps(&text) {
        if !block.contains("uses:") {
            continue;
        }
        assert!(
            !block.contains("secrets."),
            "a `uses:` step in deploy.yml is being handed a secret:\n{block}"
        );
    }
}

#[test]
fn deploy_pins_the_remote_host_key() {
    let text = fs::read_to_string(workflows_dir().join("deploy.yml")).expect("readable");

    assert!(
        text.contains("UserKnownHostsFile") && text.contains("SSH_KNOWN_HOSTS"),
        "the deploy must pin the host key from a known_hosts file"
    );
    assert!(
        !text.contains("StrictHostKeyChecking=no"),
        "a deploy that trusts whatever host answers is weaker than the action \
         it replaced"
    );
}

// ---------------------------------------------------------------------------
// Reading the workflow files
// ---------------------------------------------------------------------------

struct Uses {
    action: String,
    sha: String,
    claim: Option<String>,
    moving: bool,
}

fn workflow_files() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(workflows_dir())
        .expect(".github/workflows must exist")
        .map(|entry| entry.expect("readable dir entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
        })
        .collect();
    found.sort();
    assert!(
        !found.is_empty(),
        "expected workflows under .github/workflows"
    );
    found
}

fn workflow_uses() -> Vec<Uses> {
    let mut all = Vec::new();
    for path in workflow_files() {
        let text = fs::read_to_string(&path).expect("workflow is readable");
        for line in text.lines() {
            let trimmed = line.trim_start();
            let trimmed = trimmed.strip_prefix("- ").unwrap_or(trimmed);
            let Some(rest) = trimmed.strip_prefix("uses:") else {
                continue;
            };
            let (reference, comment) = match rest.split_once('#') {
                Some((reference, comment)) => (reference.trim(), comment.trim()),
                None => (rest.trim(), ""),
            };
            let Some((action, sha)) = reference.split_once('@') else {
                continue;
            };
            // Same grammar as scripts/verify_action_pins.py: claim first, then
            // the marker in second position. A looser rule here would build a
            // tag fixture for a pin the checker treats as moving (or the
            // reverse), and `the_current_tree_verifies` would fail with a
            // message about the wrong thing.
            let mut tokens = comment.split_whitespace();
            let claim = tokens.next().map(str::to_string);
            let moving = tokens
                .next()
                .is_some_and(|t| matches!(t.to_lowercase().as_str(), "branch" | "moving"));
            all.push(Uses {
                action: action.to_string(),
                sha: sha.to_string(),
                claim,
                moving,
            });
        }
    }
    all
}

/// Every `<scope>: <value>` under a `permissions:` key, across the file.
fn permission_scopes(path: &Path) -> Vec<(String, String)> {
    let text = fs::read_to_string(path).expect("workflow is readable");
    let mut scopes = Vec::new();
    let mut base: Option<usize> = None;

    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();

        if let Some(open) = base {
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if indent <= open {
                base = None;
            } else if let Some((scope, value)) = trimmed.split_once(':') {
                let value = value.split('#').next().unwrap_or("").trim();
                scopes.push((scope.trim().to_string(), value.to_string()));
                continue;
            }
        }
        if trimmed == "permissions:" {
            base = Some(indent);
        }
    }
    scopes
}

/// The file split at each YAML list item, so a step's `uses:` and its `with:`
/// inputs are read together.
fn steps(text: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("- ") && !blocks.is_empty() {
            blocks.push(String::new());
        }
        if blocks.is_empty() {
            blocks.push(String::new());
        }
        let last = blocks.last_mut().unwrap();
        last.push_str(line);
        last.push('\n');
    }
    blocks
}
