# Tasks

## 1. Least privilege

- [ ] 1.1 Add an explicit `permissions:` block to `ci.yml`. Both jobs read the
  repository and nothing else — `contents: read`.
- [ ] 1.2 Add one to `deploy.yml`. It checks out, builds, and rsyncs to a host;
  it needs no repository write.
- [ ] 1.3 Leave `release.yml`'s `contents: write` as it is, with its comment. It
  is the model the others are being brought up to.
- [ ] 1.4 Declare at the narrowest useful level. A workflow-level block is fine
  where every job needs the same thing; use job-level where they differ.
- [ ] 1.5 Set the repository default workflow permission to read, and disable
  workflow approval of pull requests. This is a repository setting, not a file —
  record in the docs that it is part of the configuration, or the next person
  restoring the repo will not know it was deliberate.

## 2. Pin verification

- [ ] 2.1 Add a CI job that scans every workflow for `uses:` references to
  third-party actions and verifies each one.
- [ ] 2.2 Fail on any reference that is not a full 40-hex commit SHA. A tag or
  branch reference is the thing pinning exists to prevent.
- [ ] 2.3 For each pin, resolve the version named in the trailing comment against
  the upstream repository and compare to the pinned SHA.
- [ ] 2.4 **Dereference annotated tags.** `git/ref/tags/<tag>` yields a tag object
  for annotated tags, not a commit; comparing without following
  `git/tags/<sha>` reports every correct pin as a mismatch. Verified concretely:
  `Swatinem/rust-cache` v2.9.2 resolves to tag object
  `63fed3e2fecf6f7b51dc6f043341b79ef82a9ae7`, which dereferences to commit
  `6323deb102c322ba6fcbdcafc7e3dddab59af2b6` — the pin. A check that fails on
  correct input gets muted, which is worse than no check.
- [ ] 2.5 Fail with the offending reference named, so the output is actionable
  without opening a log.
- [ ] 2.6 A pin whose comment names no version, or a version that does not exist
  upstream, fails. An unverifiable pin is not a verified one.
- [ ] 2.7 Fail when one action appears at two different SHAs across the
  workflows. A per-pin check passes both — `actions/checkout` was at v7.0.1 in
  one job and v7.0.0 in another, because the update that bumped it predated the
  job it missed, and nothing reported the split.
- [ ] 2.8 The job needs only repository read and network access to the upstream
  API; declare it accordingly per section 1.

## 3. Remove the third party from the deploy path

- [ ] 3.1 Replace the `easingthemes/ssh-deploy` step in `deploy.yml` with direct
  rsync over SSH in a `run:` step, loading the key into an agent for the life of
  the step rather than writing it to disk.
- [ ] 3.2 Preserve behavior exactly: same source (`dist/`), same target
  (`/opt/coterie/`), same pre-command ownership fix, and the same post-commands —
  install the unit file, chown, `systemctl daemon-reload`, restart.
- [ ] 3.3 Pin the remote host key rather than accepting it blindly. A deploy that
  trusts whatever host answers is a weaker link than the action it replaces, and
  swapping one for the other would be a net loss.
- [ ] 3.4 Ensure the key does not survive the step and is not written to a path
  another step could read.
- [ ] 3.5 Keep `actions/checkout` and the other pinned actions as they are — this
  removes a third party from the *credential* path, not from the workflow.

## 4. Tests and verification

- [ ] 4.1 Pin verification fails on a reference by tag; passes on the current
  tree.
- [ ] 4.2 It passes on an annotated-tag pin — the false-failure case from 2.4 —
  and this is asserted, not assumed.
- [ ] 4.3 It fails on a SHA deliberately mismatched from its comment.
- [ ] 4.4 It fails on a pin whose comment names no version.
- [ ] 4.5 It fails when one action is pinned at two SHAs across workflows, and
  passes when every occurrence agrees.
- [ ] 4.6 Confirm each workflow's token carries no write scope, except
  `release.yml`.
- [ ] 4.7 Exercise the rewritten deploy against the staging host and confirm the
  same artifacts land and the service restarts. This is a behavior-preserving
  rewrite of a step that reaches production infrastructure, so it is verified by
  running it, not by reading it.
