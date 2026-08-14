# workflow-supply-chain Specification

## Purpose
TBD - created by archiving change a63-workflow-least-privilege-and-pin-verification. Update Purpose after archive.
## Requirements
### Requirement: Workflow jobs run with the least token privilege they need

Every workflow SHALL declare the `GITHUB_TOKEN` scope it requires, and SHALL
declare no more than it uses. A job that needs nothing SHALL declare nothing.

The repository default SHALL be read-only, and workflows SHALL NOT be permitted
to approve pull requests. The explicit declarations are what each workflow relies
on; the repository default is the floor beneath them, so that a workflow added
later without a declaration is harmless rather than fully privileged.

Pinning third-party actions to commit SHAs limits **when** new third-party code
is adopted. It says nothing about what that code can reach once it is running.
With a write-scoped default token, every action in every job — a toolchain
installer, a cache action, a deploy action — can push to the repository. That
exposure is larger than the one pinning addresses and is closed by configuration
rather than by review.

Where a job legitimately needs write scope, the declaration SHALL name the
narrowest scope that works and SHALL record why, as the release workflow already
does for `contents: write`.

#### Scenario: A workflow without a stated need gets no write scope

- **WHEN** a workflow job does not declare a permission it requires
- **THEN** its token SHALL NOT carry write scope to the repository

#### Scenario: A newly added workflow is not privileged by default

- **WHEN** a workflow is added with no `permissions:` block
- **THEN** the repository default SHALL leave it read-only

#### Scenario: Workflows cannot approve pull requests

- **WHEN** any workflow attempts to approve a pull request using the workflow
  token
- **THEN** the attempt SHALL fail

### Requirement: Action pins are verified to match the version they claim

Continuous integration SHALL verify, for every third-party action referenced by a
workflow, that the reference is a full commit SHA rather than a tag or branch,
and that the SHA is the commit the version named in its adjacent comment resolves
to upstream.

The comment beside a pin is decorative — the platform resolves the SHA and never
reads the comment. A pin reading `# v3.0.2` while pointing at an unrelated commit
is indistinguishable from a correct one by inspection, and is the exact shape of
an attack against a consumer who has pinned: not a moved tag, but a pin that
never pointed where it said.

Verification SHALL dereference annotated tags before comparing. A tag reference
resolves to a tag object rather than to a commit, so comparing without
dereferencing reports every correct pin as a mismatch. A check that reports false
failures is one that gets muted, which is worse than not having it.

The check SHALL fail the build on a reference that is not a full SHA, and on a
SHA that does not match its claimed version. It SHALL name the offending
reference.

The check SHALL also fail when one action is pinned to two different versions
across the workflows. This is not hypothetical: `actions/checkout` was pinned to
`v7.0.1` in one job and `v7.0.0` in another after an update landed that predated
the job it missed. Both pins were individually valid and matched their comments,
so a per-pin check passes them both — yet one job silently runs an older action
than the repository believes it has standardized on, and the next update will
move only one of them again.

Verification SHALL NOT be satisfied by reviewing the contents of an updated
action. Reading the full diff of every action update does not scale to a
dependency's release history and therefore does not happen; a control that is not
performed provides no protection while appearing to. Depth of review remains a
human judgment about what a change would do, made against a job whose token and
secrets are already bounded by the requirement above.

#### Scenario: A pin that does not match its comment fails

- **WHEN** an action is pinned to a SHA that is not the commit its commented
  version resolves to
- **THEN** the build SHALL fail and name that reference

#### Scenario: A tag or branch reference fails

- **WHEN** an action is referenced by tag or branch rather than by full commit SHA
- **THEN** the build SHALL fail and name that reference

#### Scenario: The same action pinned to two versions fails

- **WHEN** one action is referenced at two different commits across the workflows,
  each pin individually matching its own comment
- **THEN** the build SHALL fail and name both references

#### Scenario: An annotated tag verifies correctly

- **WHEN** the claimed version is an annotated tag
- **THEN** the tag SHALL be dereferenced to its commit before comparison, and a
  correct pin SHALL pass

### Requirement: Deployment credentials are not handed to third-party actions

The deployment workflow SHALL perform its file transfer and remote commands
directly rather than passing deployment credentials to a third-party action.

The credential in question is an SSH private key for a host on which the same
step runs privileged commands — installing a systemd unit and restarting the
service. Handing that to another party's code buys the convenience of not writing
an rsync invocation, and the operation it performs is an rsync invocation.

A third party SHALL NOT be present in a credential path that the workflow can
occupy itself. This is a reduction in the number of parties trusted with the
credential, and it is not achieved by pinning, reviewing, or updating that party.

Deployment behavior SHALL be unchanged: the same files reach the same host over
the same protocol, and the same remote commands run afterwards.

#### Scenario: The deploy key is not passed to a third party

- **WHEN** the deployment workflow runs
- **THEN** the SSH private key SHALL NOT be supplied as an input to a third-party
  action

#### Scenario: Deployment still delivers the same result

- **WHEN** the deployment workflow completes
- **THEN** the same artifacts SHALL be present on the host and the service SHALL
  have been restarted, as before

