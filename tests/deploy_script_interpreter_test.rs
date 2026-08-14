//! The shipped shell scripts must never be run under an interpreter their
//! shebang contradicts.
//!
//! `release-deploy.sh` (`#!/bin/sh`) used to delegate with `exec sh
//! "$cand"`, where `$cand` was `update.sh` (`#!/usr/bin/env bash`). On
//! Debian `/bin/sh` is dash, dash has no `ERR` pseudo-signal, and every
//! update on such a host died at update.sh's `trap ... ERR` with
//! "trap: 26: bad trap". Naming an interpreter discards the shebang that
//! says which one the file needs.
//!
//! This checks the class, not that one line: any script here that names
//! `sh`/`dash` as the interpreter for another script must be able to prove
//! the target is POSIX. A target it cannot resolve (a variable, as `$cand`
//! was) is a failure — that is precisely the shape the bug had.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(PartialEq, Debug, Clone, Copy)]
enum Family {
    Posix,
    Bash,
    Unknown,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every shell script we ship. `deploy/` is where they all live; a `.sh`
/// anywhere else would need adding here (and probably a reason).
fn scripts() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(repo_root().join("deploy"))
        .expect("deploy/ must exist")
        .map(|e| e.expect("readable dir entry").path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "sh"))
        .collect();
    found.sort();
    assert!(!found.is_empty(), "expected shell scripts under deploy/");
    found
}

fn family_of_shebang(script: &Path) -> Family {
    let text = fs::read_to_string(script).expect("script is readable text");
    let first = text.lines().next().unwrap_or_default();
    match first {
        l if !l.starts_with("#!") => Family::Unknown,
        l if l.contains("bash") => Family::Bash,
        l if l.contains("sh") => Family::Posix,
        _ => Family::Unknown,
    }
}

fn family_of_interpreter(token: &str) -> Option<Family> {
    match token.rsplit('/').next().unwrap_or(token) {
        "sh" | "dash" | "ash" => Some(Family::Posix),
        "bash" => Some(Family::Bash),
        _ => None,
    }
}

fn unquote(token: &str) -> &str {
    token.trim_matches(|c| c == '"' || c == '\'')
}

#[test]
fn no_script_is_invoked_with_an_interpreter_its_shebang_contradicts() {
    let all = scripts();
    let mut problems: Vec<String> = Vec::new();

    for script in &all {
        let text = fs::read_to_string(script).expect("script is readable text");
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let tokens: Vec<&str> = line.split_whitespace().collect();
            for pair in tokens.windows(2) {
                let Some(invoker) = family_of_interpreter(unquote(pair[0])) else {
                    continue;
                };
                let target = unquote(pair[1]);
                // `sh -c '...'`, `bash --norc`: not a script reference.
                if target.starts_with('-') {
                    continue;
                }
                // A script path, or a variable holding one. Anything else
                // (`sha256sum -c`, a bare word) is not this pattern.
                let is_var = target.contains('$');
                if !target.ends_with(".sh") && !is_var {
                    continue;
                }
                let named = target.rsplit('/').next().unwrap_or(target);
                let resolved = all
                    .iter()
                    .find(|p| p.file_name().is_some_and(|f| f == named));
                let target_family = match resolved {
                    Some(p) => family_of_shebang(p),
                    None => Family::Unknown,
                };
                // bash runs POSIX scripts fine; the reverse is the defect.
                if invoker == Family::Posix && target_family != Family::Posix {
                    problems.push(format!(
                        "{}:{}: invokes `{}` with `{}` — target is {:?}, so the shebang \
                         is discarded (dash has no ERR trap). Exec the file itself, or \
                         name bash.",
                        script.display(),
                        n + 1,
                        target,
                        pair[0],
                        target_family,
                    ));
                }
            }
        }
    }

    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Placement has to keep these runnable: `release-deploy.sh` execs
/// `update.sh` directly, `coterie-provision update` chmods 0755 on
/// refresh, and the release tarball carries whatever mode git records.
#[test]
fn shipped_scripts_are_executable_and_declare_a_shebang() {
    for script in scripts() {
        assert_ne!(
            family_of_shebang(&script),
            Family::Unknown,
            "{} has no recognizable shebang",
            script.display()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&script)
                .expect("script metadata")
                .permissions()
                .mode();
            assert!(
                mode & 0o111 != 0,
                "{} is not executable (mode {:o}) — it is exec'd directly",
                script.display(),
                mode
            );
        }
    }
}
