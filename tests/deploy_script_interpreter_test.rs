//! Deploy scripts run under the interpreter their shebang names.
//!
//! `release-deploy.sh` used to hand `update.sh` to `sh`. `update.sh` is
//! bash and `/bin/sh` is dash on Debian, so dash hit its `trap ... ERR`
//! — a bash pseudo-signal — and aborted with `trap: 26: bad trap`,
//! deploying nothing. The interpreter is the script's own business; the
//! shebang states it.
//!
//! This drives the real `release-deploy.sh` against a fake install dir
//! (`COTERIE_INSTALL_DIR`) with a stub updater, because the defect was
//! `exec sh "$cand"` — the target was a variable, so no amount of
//! reading the literal text would have caught it. The static check at
//! the bottom covers the same class where the target IS literal.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use uuid::Uuid;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A temp dir that removes itself even when an assertion blows up.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("coterie-deploy-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_exec(path: &Path, body: &str) {
    fs::write(path, body).expect("write script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// A stand-in for `update.sh` carrying the header that actually broke:
/// `set -o pipefail` and a `trap ... ERR` are both bash-only, so under
/// dash this stub dies exactly as the real one did.
const UPDATE_STUB: &str = r#"#!/usr/bin/env bash
set -euo pipefail
trap 'echo "[stub] ERROR on line $LINENO" >&2' ERR
echo "STUB=update.sh"
echo "STUB_INTERPRETER=${BASH_VERSION:+bash}"
echo "STUB_ARGS=$*"
"#;

/// `$INSTALL_DIR/coterie` present + a stub `update.sh` beside the copied
/// `release-deploy.sh` — i.e. the update path with no provision binary.
fn stage(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let install = tmp.path().join("install");
    let deploy = tmp.path().join("deploy");
    fs::create_dir_all(&install).unwrap();
    fs::create_dir_all(&deploy).unwrap();
    fs::write(install.join("coterie"), b"pretend-binary").unwrap();

    let script = deploy.join("release-deploy.sh");
    fs::copy(repo_root().join("deploy/release-deploy.sh"), &script)
        .expect("copy release-deploy.sh");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    write_exec(&deploy.join("update.sh"), UPDATE_STUB);
    (install, script)
}

fn run(script: &Path, install: &Path, args: &[&str]) -> Output {
    Command::new(script)
        .args(args)
        .env("COTERIE_INSTALL_DIR", install)
        .output()
        .expect("run release-deploy.sh")
}

#[test]
fn bootstrap_fallback_runs_update_sh_under_bash_and_says_it_fell_back() {
    let tmp = TempDir::new();
    let (install, script) = stage(&tmp);

    let out = run(&script, &install, &["v1.2.3"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "release-deploy.sh failed\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("STUB_INTERPRETER=bash"),
        "update.sh must run under bash, not sh/dash\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("STUB_ARGS=--tag v1.2.3"),
        "the positional tag is forwarded as --tag\nstdout: {stdout}"
    );
    // The fallback means the hardened path was unavailable. Silence here
    // is what let every update on the production host take the broken
    // path unnoticed.
    assert!(
        stderr.contains("no executable coterie-provision at"),
        "the missing provision binary is reported\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("falling back to the bootstrap updater"),
        "taking the bootstrap path is announced\nstderr: {stderr}"
    );
}

/// An install placed before the exec bit was preserved still updates —
/// it does not silently regress into being run by `sh`.
#[test]
fn update_sh_without_its_exec_bit_still_runs_under_bash() {
    let tmp = TempDir::new();
    let (install, script) = stage(&tmp);
    let update = tmp.path().join("deploy/update.sh");
    fs::set_permissions(&update, fs::Permissions::from_mode(0o644)).unwrap();

    let out = run(&script, &install, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("STUB_INTERPRETER=bash"),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// When the binary IS on the box, the hardened path is the one taken —
/// the bootstrap is a fallback, not the norm.
#[test]
fn an_installed_coterie_provision_is_preferred_over_the_bootstrap() {
    let tmp = TempDir::new();
    let (install, script) = stage(&tmp);
    write_exec(
        &install.join("coterie-provision"),
        "#!/bin/sh\necho \"PROVISION_ARGS=$*\"\n",
    );

    let out = run(&script, &install, &["v1.2.3"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout: {stdout}");
    assert!(
        stdout.contains("PROVISION_ARGS=update --tag v1.2.3"),
        "coterie-provision update is the delegation target\nstdout: {stdout}"
    );
    assert!(
        !stdout.contains("STUB=update.sh"),
        "the bootstrap must not run when the binary is present\nstdout: {stdout}"
    );
}

/// The static half of the same rule: where a deploy script names another
/// script literally, the interpreter it names must match that script's
/// shebang — and a script handed to a hardcoded interpreter through a
/// variable can't be checked at all, so it must be exec'd directly.
#[test]
fn no_deploy_script_is_run_by_an_interpreter_its_shebang_contradicts() {
    let deploy = repo_root().join("deploy");
    let scripts: Vec<PathBuf> = fs::read_dir(&deploy)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "sh"))
        .collect();
    assert!(scripts.len() > 3, "expected the deploy scripts to be found");

    // Shebang family per script: bash, or POSIX sh.
    let family = |text: &str| {
        if text.lines().next().unwrap_or_default().contains("bash") {
            "bash"
        } else {
            "sh"
        }
    };
    let shebangs: Vec<(String, &'static str)> = scripts
        .iter()
        .map(|p| {
            let text = fs::read_to_string(p).unwrap();
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                family(&text),
            )
        })
        .collect();

    for script in &scripts {
        let text = fs::read_to_string(script).unwrap();
        let name = script.file_name().unwrap().to_string_lossy();
        for (n, line) in text.lines().enumerate() {
            let mut tokens = line.split_whitespace().peekable();
            if tokens.peek() == Some(&"#") || line.trim_start().starts_with('#') {
                continue;
            }
            if tokens.peek() == Some(&"exec") {
                tokens.next();
            }
            let Some(interp) = tokens.next() else {
                continue;
            };
            let interp = interp.rsplit('/').next().unwrap_or(interp);
            let interp = match interp {
                "bash" => "bash",
                "sh" | "dash" => "sh",
                _ => continue,
            };
            let Some(target) = tokens.next() else {
                continue;
            };
            let at = format!("{name}:{}: {}", n + 1, line.trim());

            let named = shebangs.iter().find(|(n, _)| target.contains(n.as_str()));
            match named {
                Some((_, wants)) => assert_eq!(
                    interp, *wants,
                    "{at}\n  runs a {wants} script with `{interp}`"
                ),
                None => assert!(
                    !target.contains('$'),
                    "{at}\n  hands a script named by a variable to `{interp}`; \
                     exec it directly so its shebang picks the interpreter"
                ),
            }
        }
    }
}

/// Whatever places these files, the exec bit has to be on them — the
/// tarball carries the repo's modes, and `release-deploy.sh` execs
/// `update.sh` directly.
#[test]
fn deploy_shell_scripts_are_executable_in_the_repo() {
    for entry in fs::read_dir(repo_root().join("deploy")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "sh") {
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert!(
                mode & 0o111 != 0,
                "{} is not executable (mode {mode:o})",
                path.display()
            );
        }
    }
}
