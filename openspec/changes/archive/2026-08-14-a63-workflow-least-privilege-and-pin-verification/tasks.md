# Tasks

## 1. Least privilege

- [x] 1.1 Add an explicit `permissions:` block to `ci.yml`. Both jobs read the
  repository and nothing else — `contents: read`.
- [x] 1.2 Add one to `deploy.yml`. It checks out, builds, and rsyncs to a host;
  it needs no repository write.
- [x] 1.3 Leave `release.yml`'s `contents: write` as it is, with its comment. It
  is the model the others are being brought up to.
- [x] 1.4 Declare at the narrowest useful level. A workflow-level block is fine
  where every job needs the same thing; use job-level where they differ.
- [x] 1.5 Record the repository-level half of this in `docs/deploy/OPS.md`: the
  default workflow permission is read-only and "Allow GitHub Actions to create
  and approve pull requests" is off, both deliberate, both to be restored if the
  repository is ever recreated. The settings themselves were applied on
  2026-08-13 and are already in effect — this task is the record, so a future
  reader knows the state is intended rather than accidental. Do not attempt to
  change repository settings from a run; verify nothing and assume nothing about
  them.

## 2. Pin verification

- [x] 2.1 Add a CI job that scans every workflow for `uses:` references to
  third-party actions and verifies each one.
- [x] 2.2 Fail on any reference that is not a full 40-hex commit SHA. A tag or
  branch reference is the thing pinning exists to prevent.
- [x] 2.3 For each pin, resolve the version named in the trailing comment against
  the upstream repository and compare to the pinned SHA.
- [x] 2.4 Branch out on what the comment names. Ten of the fourteen pins in this
  repository name a tag; four name `stable`, a branch. For a **tag** claim,
  assert equality with the (dereferenced) tag commit. For a **moving reference**
  — a branch, or a tag upstream republishes — assert only that the commit exists
  in the upstream repository.
- [x] 2.5 Do NOT assert equality or ancestry for a moving reference. Equality
  fails on every upstream push and pressures the pin to chase the branch, which
  is the tracking pinning exists to avoid. Ancestry fails too when upstream
  rebases: `dtolnay/rust-toolchain`'s `stable` reports `diverged` against the
  commit pinned here (ahead 4, behind 1) while that commit is still present in
  the repository. Verified against the live API, not assumed.
- [x] 2.6 Apply the strongest assertion each claim admits. Do not lower tag
  claims to existence checks because branch claims need one — that would discard
  the check's value for the majority of pins to accommodate the minority.
- [x] 2.7 **Dereference annotated tags.** `git/ref/tags/<tag>` yields a tag object
  for annotated tags, not a commit; comparing without following
  `git/tags/<sha>` reports every correct pin as a mismatch. Verified concretely:
  `Swatinem/rust-cache` v2.9.2 resolves to tag object
  `63fed3e2fecf6f7b51dc6f043341b79ef82a9ae7`, which dereferences to commit
  `6323deb102c322ba6fcbdcafc7e3dddab59af2b6` — the pin. A check that fails on
  correct input gets muted, which is worse than no check.
- [x] 2.8 Fail with the offending reference named, so the output is actionable
  without opening a log.
- [x] 2.9 A pin whose comment names no version, or a version that does not exist
  upstream, fails. An unverifiable pin is not a verified one.
- [x] 2.10 Fail when one action appears at two different SHAs across the
  workflows. A per-pin check passes both — `actions/checkout` was at v7.0.1 in
  one job and v7.0.0 in another, because the update that bumped it predated the
  job it missed, and nothing reported the split.
- [x] 2.11 The job needs only repository read and network access to the upstream
  API; declare it accordingly per section 1.

## 3. Remove the third party from the deploy path

- [x] 3.1 Replace the `easingthemes/ssh-deploy` step in `deploy.yml` with direct
  rsync over SSH in a `run:` step, loading the key into an agent for the life of
  the step rather than writing it to disk.
- [x] 3.2 Preserve behavior exactly: same source (`dist/`), same target
  (`/opt/coterie/`), same pre-command ownership fix, and the same post-commands —
  install the unit file, chown, `systemctl daemon-reload`, restart.
- [x] 3.3 Pin the remote host key rather than accepting it blindly. A deploy that
  trusts whatever host answers is a weaker link than the action it replaces, and
  swapping one for the other would be a net loss. Write the workflow to consume a
  `SSH_KNOWN_HOSTS` repository secret and write it to a `known_hosts` file for the
  step; do NOT use `StrictHostKeyChecking=no`. A host's public key is not itself
  secret, but obtaining it requires reaching the host, which a run cannot do — so
  the workflow consumes the value and `docs/deploy/OPS.md` records how it is
  produced (`ssh-keyscan` against the deploy host) and that the deploy fails
  until it exists. Failing closed is correct here.
- [x] 3.4 Ensure the key does not survive the step and is not written to a path
  another step could read.
- [x] 3.5 Where a pin names a moving reference, extend its comment to say so and
  to record when the pin was taken, so a reader is not left inferring why the
  check treats it differently.
- [x] 3.6 Keep `actions/checkout` and the other pinned actions as they are — this
  removes a third party from the *credential* path, not from the workflow.

## 4. Tests and static verification

- [x] 4.1 Pin verification fails on a reference by tag; passes on the current
  tree.
- [x] 4.2 It passes on an annotated-tag pin — the false-failure case from 2.4 —
  and this is asserted, not assumed.
- [x] 4.3 It fails on a SHA deliberately mismatched from its comment.
- [x] 4.4 It fails on a pin whose comment names no version.
- [x] 4.5 It fails when one action is pinned at two SHAs across workflows, and
  passes when every occurrence agrees.
- [x] 4.6 A branch-claim pin whose commit exists upstream passes even when the
  branch has moved or been rebased past it — the live `dtolnay/rust-toolchain`
  case, which failed the first implementation.
- [x] 4.7 A branch-claim pin whose commit is absent upstream fails.
- [x] 4.8 A tag-claim pin with a mismatched SHA still fails; existence upstream
  does not rescue it.
- [x] 4.9 Confirm each workflow's token carries no write scope, except
  `release.yml`.
- [x] 4.10 Assert statically that `deploy.yml` hands no secret to a third party:
  fail if any `uses:` step in that workflow carries a `with:` input referencing
  `secrets.`, and assert the rsync step configures a `known_hosts` file rather
  than disabling host-key checking. A run cannot reach the deploy host, so the
  property is asserted against the workflow file — which is where the property
  actually lives.
- [x] 4.11 Make the pin resolver injectable so the checks in 4.1–4.5 run offline
  against fixtures, with the live upstream API used only when CI runs it. A test
  that needs network is a test that gets skipped.

## 5. Documentation

- [x] 5.1 In `docs/deploy/OPS.md`, record the post-merge deploy verification: run
  the staging deploy via `workflow_dispatch`, confirm `/opt/coterie` is updated
  and `systemctl status coterie` shows the service restarted. This is a
  behavior-preserving rewrite of a step that reaches real infrastructure, so it
  wants a human confirmation once — recorded where operators look, not as a
  checkbox in a change that will be archived.
- [x] 5.2 Same file: note that `SSH_KNOWN_HOSTS` must exist before the rewritten
  deploy can succeed, and how to produce it.
