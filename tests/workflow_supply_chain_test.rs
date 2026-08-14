//! The workflows' supply-chain properties, checked statically.
//!
//! Three things are asserted here, none of which the platform enforces:
//!
//! 1. `.github/scripts/verify-action-pins.sh` accepts correct pins and
//!    rejects the ways a pin goes wrong — a tag reference, a SHA that is not
//!    the version its comment claims, a comment claiming nothing, and one
//!    action pinned at two SHAs across workflows.
//! 2. No workflow's token carries write scope, except the release workflow,
//!    which says why it needs `contents: write`.
//! 3. `deploy.yml` hands no secret to a third-party action, and pins the
//!    remote host key instead of accepting whatever answers.
//!
//! The pin checker talks to api.github.com through one seam — `api()`, keyed
//! on `PIN_API_FIXTURES` — so every case below runs offline against fixture
//! JSON shaped like the real responses. A test that needs network is a test
//! that gets skipped, and the annotated-tag case (the one that reports correct
//! pins as broken when nobody dereferences) is exactly the one worth asserting
//! rather than assuming.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use uuid::Uuid;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn checker() -> PathBuf {
    repo_root().join(".github/scripts/verify-action-pins.sh")
}

fn workflow_dir() -> PathBuf {
    repo_root().join(".github/workflows")
}

/// A temp dir that removes itself even when an assertion blows up.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("coterie-pins-{}", Uuid::new_v4()));
        fs::create_dir_all(path.join("workflows")).expect("create temp dir");
        fs::create_dir_all(path.join("fixtures")).expect("create temp dir");
        Self(path)
    }
    fn workflows(&self) -> PathBuf {
        self.0.join("workflows")
    }
    fn fixtures(&self) -> PathBuf {
        self.0.join("fixtures")
    }

    fn workflow(&self, name: &str, body: &str) {
        fs::write(self.workflows().join(name), body).expect("write workflow");
    }

    /// A lightweight tag: `git/ref/tags/<tag>` points straight at the commit.
    fn lightweight_tag(&self, repo: &str, tag: &str, commit: &str) {
        self.fixture(
            &format!("repos/{repo}/git/ref/tags/{tag}"),
            &format!(r#"{{"object":{{"type":"commit","sha":"{commit}"}}}}"#),
        );
    }

    /// An annotated tag: the ref yields a tag OBJECT, which has to be
    /// dereferenced through `git/tags/<sha>` before it names a commit.
    fn annotated_tag(&self, repo: &str, tag: &str, tag_object: &str, commit: &str) {
        self.fixture(
            &format!("repos/{repo}/git/ref/tags/{tag}"),
            &format!(r#"{{"object":{{"type":"tag","sha":"{tag_object}"}}}}"#),
        );
        self.fixture(
            &format!("repos/{repo}/git/tags/{tag_object}"),
            &format!(r#"{{"object":{{"type":"commit","sha":"{commit}"}}}}"#),
        );
    }

    /// A branch, plus the commits that are on it.
    fn branch(&self, repo: &str, branch: &str, head: &str, members: &[&str]) {
        self.fixture(
            &format!("repos/{repo}/git/ref/heads/{branch}"),
            &format!(r#"{{"object":{{"type":"commit","sha":"{head}"}}}}"#),
        );
        for member in members {
            let status = if *member == head { "identical" } else { "behind" };
            self.fixture(
                &format!("repos/{repo}/compare/{branch}...{member}"),
                &format!(r#"{{"status":"{status}"}}"#),
            );
        }
    }

    fn fixture(&self, api_path: &str, body: &str) {
        let name = format!("{}.json", api_path.replace('/', "__"));
        fs::write(self.fixtures().join(name), body).expect("write fixture");
    }

    /// Run the checker over this dir's workflows. Returns (passed, output).
    fn verify(&self) -> (bool, String) {
        run_checker(&self.workflows(), &self.fixtures())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_checker(workflows: &Path, fixtures: &Path) -> (bool, String) {
    let out = Command::new("bash")
        .arg(checker())
        .arg(workflows)
        .env("PIN_API_FIXTURES", fixtures)
        .output()
        .expect("run verify-action-pins.sh");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

// ---------------------------------------------------------------------------
// Reading the real workflows
// ---------------------------------------------------------------------------

struct Pin {
    file: String,
    repo: String,
    sha: String,
    version: String,
}

/// Every `uses:` reference in a workflow file, as (repo, ref, comment).
/// Deliberately the same shape the checker parses, so the fixtures the tests
/// generate below describe the pins that are actually there.
fn pins_in(path: &Path) -> Vec<Pin> {
    let file = path.file_name().unwrap().to_string_lossy().into_owned();
    let body = fs::read_to_string(path).expect("read workflow");
    let mut pins = Vec::new();
    for line in body.lines() {
        let t = line.trim_start();
        let t = t.strip_prefix("- ").map(str::trim_start).unwrap_or(t);
        let Some(rest) = t.strip_prefix("uses:") else {
            continue;
        };
        let rest = rest.trim();
        let (reference, comment) = match rest.split_once('#') {
            Some((r, c)) => (r.trim(), c.trim()),
            None => (rest, ""),
        };
        if reference.starts_with("./") || reference.starts_with("docker://") {
            continue;
        }
        let (action, sha) = reference.split_once('@').unwrap_or((reference, ""));
        let repo = action.split('/').take(2).collect::<Vec<_>>().join("/");
        let version = comment
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches([',', '.', ';', ':'])
            .to_string();
        pins.push(Pin {
            file: file.clone(),
            repo,
            sha: sha.to_string(),
            version,
        });
    }
    pins
}

fn real_workflows() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(workflow_dir())
        .expect("read .github/workflows")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no workflow files found");
    files
}

fn read_workflow(name: &str) -> String {
    fs::read_to_string(workflow_dir().join(name)).expect("read workflow")
}

/// Strip a trailing `# comment` from a YAML line.
fn without_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim_end()
}

// ---------------------------------------------------------------------------
// Pin verification (tasks 4.1-4.5, 4.8)
// ---------------------------------------------------------------------------

/// The current tree passes: every pin is a full SHA carrying a version, and no
/// action is split across two SHAs. Upstream truth is supplied by fixtures
/// built from the pins themselves — whether each SHA really is that version
/// upstream is what the CI job answers, since that half needs the network.
#[test]
fn current_workflows_pass_pin_verification() {
    let tmp = TempDir::new();
    let mut count = 0;

    for path in real_workflows() {
        fs::copy(&path, tmp.workflows().join(path.file_name().unwrap())).expect("copy workflow");
        for pin in pins_in(&path) {
            count += 1;
            assert_eq!(
                pin.sha.len(),
                40,
                "{} pins {} to {:?}, which is not a full commit SHA",
                pin.file,
                pin.repo,
                pin.sha
            );
            assert!(
                !pin.version.is_empty(),
                "{} pins {} with no version named in its comment",
                pin.file,
                pin.repo
            );
            // A version-shaped comment names a tag; anything else (this repo
            // has one action published only from a branch) names a branch.
            if pin.version.trim_start_matches('v').starts_with(|c: char| c.is_ascii_digit()) {
                tmp.lightweight_tag(&pin.repo, &pin.version, &pin.sha);
            } else {
                tmp.branch(&pin.repo, &pin.version, &pin.sha, &[&pin.sha]);
            }
        }
    }

    assert!(count > 0, "no action pins found in the workflows");
    let (passed, output) = tmp.verify();
    assert!(passed, "current workflows failed pin verification:\n{output}");
}

#[test]
fn tag_reference_fails() {
    let tmp = TempDir::new();
    tmp.workflow("ci.yml", "    steps:\n      - uses: actions/checkout@v7.0.1\n");
    tmp.lightweight_tag("actions/checkout", "v7.0.1", &"a".repeat(40));

    let (passed, output) = tmp.verify();
    assert!(!passed, "a tag reference must fail:\n{output}");
    assert!(
        output.contains("actions/checkout@v7.0.1") && output.contains("full 40-hex"),
        "failure must name the reference:\n{output}"
    );
}

/// The false-failure case: `git/ref/tags/v2.9.2` yields the tag object, not the
/// commit. Real values from Swatinem/rust-cache v2.9.2 — tag object
/// `63fed3e2…`, dereferencing to commit `6323deb1…`, which is the pin.
#[test]
fn annotated_tag_pin_passes() {
    let tag_object = "63fed3e2fecf6f7b51dc6f043341b79ef82a9ae7";
    let commit = "6323deb102c322ba6fcbdcafc7e3dddab59af2b6";

    let tmp = TempDir::new();
    tmp.workflow(
        "ci.yml",
        &format!("    steps:\n      - uses: Swatinem/rust-cache@{commit} # v2.9.2\n"),
    );
    tmp.annotated_tag("Swatinem/rust-cache", "v2.9.2", tag_object, commit);

    let (passed, output) = tmp.verify();
    assert!(
        passed,
        "an annotated tag must be dereferenced before comparing:\n{output}"
    );

    // And the dereference is a real comparison, not a rubber stamp: the tag
    // object's own SHA is not an acceptable pin.
    let tmp = TempDir::new();
    tmp.workflow(
        "ci.yml",
        &format!("    steps:\n      - uses: Swatinem/rust-cache@{tag_object} # v2.9.2\n"),
    );
    tmp.annotated_tag("Swatinem/rust-cache", "v2.9.2", tag_object, commit);
    let (passed, _) = tmp.verify();
    assert!(!passed, "the tag object's SHA is not the commit");
}

#[test]
fn lightweight_tag_pin_passes() {
    let commit = "3d3c42e5aac5ba805825da76410c181273ba90b1";
    let tmp = TempDir::new();
    tmp.workflow(
        "ci.yml",
        &format!("    steps:\n      - uses: actions/checkout@{commit} # v7.0.1\n"),
    );
    tmp.lightweight_tag("actions/checkout", "v7.0.1", commit);

    let (passed, output) = tmp.verify();
    assert!(passed, "a correct lightweight-tag pin must pass:\n{output}");
}

#[test]
fn sha_mismatched_from_its_comment_fails() {
    let tmp = TempDir::new();
    let pinned = "1111111111111111111111111111111111111111";
    let real = "2222222222222222222222222222222222222222";
    tmp.workflow(
        "ci.yml",
        &format!("    steps:\n      - uses: actions/checkout@{pinned} # v7.0.1\n"),
    );
    tmp.lightweight_tag("actions/checkout", "v7.0.1", real);

    let (passed, output) = tmp.verify();
    assert!(!passed, "a pin that lies about its version must fail");
    assert!(
        output.contains(pinned) && output.contains(real),
        "failure must name the reference and what the version really is:\n{output}"
    );
}

#[test]
fn pin_without_a_version_comment_fails() {
    let sha = "3d3c42e5aac5ba805825da76410c181273ba90b1";

    for line in [
        format!("      - uses: actions/checkout@{sha}"),
        format!("      - uses: actions/checkout@{sha} #"),
    ] {
        let tmp = TempDir::new();
        tmp.workflow("ci.yml", &format!("    steps:\n{line}\n"));
        tmp.lightweight_tag("actions/checkout", "v7.0.1", sha);

        let (passed, output) = tmp.verify();
        assert!(!passed, "an unverifiable pin is not a verified one: {line}");
        assert!(
            output.contains("no version named"),
            "failure must say what is missing:\n{output}"
        );
    }
}

#[test]
fn version_that_does_not_exist_upstream_fails() {
    let sha = "3d3c42e5aac5ba805825da76410c181273ba90b1";
    let tmp = TempDir::new();
    tmp.workflow(
        "ci.yml",
        &format!("    steps:\n      - uses: actions/checkout@{sha} # v9.9.9\n"),
    );
    // No fixture for v9.9.9: the lookup 404s.

    let (passed, output) = tmp.verify();
    assert!(!passed, "a version that does not exist upstream must fail");
    assert!(
        output.contains("v9.9.9") && output.contains("does not resolve"),
        "failure must name the claimed version:\n{output}"
    );
}

/// Both pins match their own comments, so a per-pin check passes both — yet
/// one job runs an older action than the repository thinks it standardized on.
#[test]
fn one_action_at_two_shas_across_workflows_fails() {
    let old = "9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0";
    let new = "3d3c42e5aac5ba805825da76410c181273ba90b1";

    let tmp = TempDir::new();
    tmp.workflow(
        "ci.yml",
        &format!("    steps:\n      - uses: actions/checkout@{new} # v7.0.1\n"),
    );
    tmp.workflow(
        "deploy.yml",
        &format!("    steps:\n      - uses: actions/checkout@{old} # v7.0.0\n"),
    );
    tmp.lightweight_tag("actions/checkout", "v7.0.1", new);
    tmp.lightweight_tag("actions/checkout", "v7.0.0", old);

    let (passed, output) = tmp.verify();
    assert!(!passed, "a split pin must fail even when both halves verify");
    assert!(
        output.contains(old) && output.contains(new),
        "failure must name both references:\n{output}"
    );

    // And agreeing occurrences pass.
    let tmp = TempDir::new();
    tmp.workflow(
        "ci.yml",
        &format!("    steps:\n      - uses: actions/checkout@{new} # v7.0.1\n"),
    );
    tmp.workflow(
        "deploy.yml",
        &format!("    steps:\n      - uses: actions/checkout@{new} # v7.0.1\n"),
    );
    tmp.lightweight_tag("actions/checkout", "v7.0.1", new);
    let (passed, output) = tmp.verify();
    assert!(passed, "agreeing pins must pass:\n{output}");
}

/// An action published only from a branch (no release tags) can't be compared
/// against a fixed commit — the head moves, which is why the SHA is pinned.
/// "This commit is on that branch" is the same claim, and it is checkable.
#[test]
fn branch_pinned_action_must_be_on_that_branch() {
    let head = "5555555555555555555555555555555555555555";
    let pinned = "4be7066ada62dd38de10e7b70166bc74ed198c30";
    let elsewhere = "6666666666666666666666666666666666666666";

    let tmp = TempDir::new();
    tmp.workflow(
        "ci.yml",
        &format!("    steps:\n      - uses: dtolnay/rust-toolchain@{pinned} # stable branch, pinned\n"),
    );
    tmp.branch("dtolnay/rust-toolchain", "stable", head, &[pinned, head]);
    let (passed, output) = tmp.verify();
    assert!(passed, "a commit on the named branch must pass:\n{output}");

    let tmp = TempDir::new();
    tmp.workflow(
        "ci.yml",
        &format!("    steps:\n      - uses: dtolnay/rust-toolchain@{elsewhere} # stable branch, pinned\n"),
    );
    tmp.branch("dtolnay/rust-toolchain", "stable", head, &[pinned, head]);
    let (passed, output) = tmp.verify();
    assert!(!passed, "a commit that is not on the branch must fail");
    assert!(output.contains(elsewhere), "failure must name it:\n{output}");
}

// ---------------------------------------------------------------------------
// Least privilege (task 4.6)
// ---------------------------------------------------------------------------

/// Every workflow states its token scope, and only the release workflow asks
/// for write — it creates releases, and says so beside the declaration.
#[test]
fn no_workflow_token_carries_write_scope_except_release() {
    for path in real_workflows() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let body = fs::read_to_string(&path).expect("read workflow");

        assert!(
            body.lines().any(|l| l.trim() == "permissions:"),
            "{name} declares no permissions block, so it inherits whatever the \
             repository default happens to be"
        );

        let writes: Vec<&str> = body
            .lines()
            .map(without_comment)
            .filter(|l| l.trim_end().ends_with(": write") || l.trim() == "permissions: write-all")
            .collect();

        if name == "release.yml" {
            assert_eq!(
                writes.iter().map(|l| l.trim()).collect::<Vec<_>>(),
                vec!["contents: write"],
                "release.yml should need exactly contents: write"
            );
        } else {
            assert!(
                writes.is_empty(),
                "{name} grants write scope: {writes:?}"
            );
        }
    }

    for name in ["ci.yml", "deploy.yml"] {
        assert!(
            read_workflow(name).contains("contents: read"),
            "{name} should declare contents: read"
        );
    }
}

// ---------------------------------------------------------------------------
// The deploy credential path (task 4.7)
// ---------------------------------------------------------------------------

/// A run cannot reach the deploy host, so the property is asserted against the
/// workflow file — which is where the property actually lives.
#[test]
fn deploy_hands_no_secret_to_a_third_party() {
    let body = read_workflow("deploy.yml");

    // Step blocks: a step starts at `- ` at the steps indent level.
    let mut steps: Vec<Vec<&str>> = Vec::new();
    for line in body.lines() {
        if line.trim_start().starts_with("- ") && line.starts_with("      ") {
            steps.push(Vec::new());
        }
        if let Some(step) = steps.last_mut() {
            step.push(line);
        }
    }
    assert!(!steps.is_empty(), "deploy.yml has no steps?");

    for step in &steps {
        let uses = step.iter().find(|l| l.trim_start().starts_with("uses:"));
        let Some(uses) = uses else { continue };
        let secret = step.iter().find(|l| l.contains("secrets."));
        assert!(
            secret.is_none(),
            "deploy.yml passes a secret to a third-party action:\n{}\n{}",
            uses.trim(),
            secret.unwrap().trim()
        );
    }

    // The transfer is a run: step, and it pins the host rather than trusting
    // whatever answers on that address.
    assert!(
        body.contains("rsync -rlgoDzvc"),
        "the deploy should rsync directly"
    );
    assert!(
        body.contains("UserKnownHostsFile") && body.contains("SSH_KNOWN_HOSTS"),
        "the deploy should pin the remote host key from SSH_KNOWN_HOSTS"
    );
    assert!(
        !body.contains("StrictHostKeyChecking=no"),
        "the deploy must not accept an unknown host key"
    );
    assert!(
        body.contains("StrictHostKeyChecking=yes"),
        "host-key checking should be explicit"
    );

    // The key lives in an agent for the life of the step, not on disk.
    assert!(
        body.contains("ssh-agent") && body.contains("ssh-add -"),
        "the private key should be loaded into an ssh-agent, not written out"
    );
}
