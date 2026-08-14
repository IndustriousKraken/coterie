# Change: CI workflows run least-privileged, and their action pins are verified

## Why

Every third-party action in this repository is pinned to a 40-character commit
SHA, deliberately, so that a moved upstream tag cannot change what runs. That
control works. Two things around it do not.

**The token those actions receive is far larger than any of them needs.** The
repository's default workflow permission is `write`, and only `release.yml`
declares a `permissions:` block. So every step in `ci.yml` and `deploy.yml` —
including `dtolnay/rust-toolchain`, `Swatinem/rust-cache`, `actions/cache`, and
`easingthemes/ssh-deploy` — runs with a `GITHUB_TOKEN` that can write to the
repository. The repository also permits workflows to approve pull requests.

Pinning limits *when* new third-party code is adopted. It does nothing about what
that code can reach once running. A compromised action today can push to the
repository; that is a larger exposure than the one pinning addresses, and it is
two settings rather than a review process.

**Nothing checks that a pin is what it claims to be.** The SHA sits next to a
`# vX.Y.Z` comment that is purely decorative — GitHub resolves the SHA and
ignores the comment. A pin that says `# v3.0.2` while pointing at an unrelated
commit is indistinguishable by eye, and that is precisely the shape of a
supply-chain attack against a pinned consumer.

The verification is cheap and mechanical: resolve the tag named in the comment
and compare. Doing it by hand has a trap — for annotated tags,
`git/ref/tags/<tag>` returns the tag object rather than the commit, so a correct
pin looks wrong until dereferenced. Every current pin was checked this way and
all four pending updates matched, but nothing makes that a repeatable property.

**And one action sits in a credential path it does not need to be in.**
`deploy.yml` hands `easingthemes/ssh-deploy` an SSH private key for a host where
the same step then runs `sudo chown`, `sudo cp` into `/etc/systemd/system/`, and
`sudo systemctl restart` — root-equivalent access. What the action does is rsync
over SSH, which is a shell command. A third party in that path is a trust
relationship bought for convenience.

## What Changes

- **Workflows declare least-privilege permissions explicitly.** Each workflow, or
  each job, states the token scope it needs; the default is none. `release.yml`
  already models this with `contents: write` and a comment saying why.
- **The repository default becomes read-only**, and workflow approval of pull
  requests is disabled. The explicit blocks are what the workflows rely on; the
  repository default is the floor beneath them, so that a workflow added later
  without a `permissions:` block is harmless rather than fully privileged.
- **CI verifies every action pin**: that each `uses:` reference is a full commit
  SHA and not a tag or branch, and that the SHA matches the tag its comment
  names. Annotated tags are dereferenced, because not doing so reports every
  correct pin as a mismatch and a check that cries wolf gets muted.
- **The deploy step stops using a third-party action.** Rsync over SSH is
  performed directly, so the SSH key is handled by the workflow rather than
  passed to somebody else's code.

## Why verify the pin rather than review the diff

Reviewing the full diff of every action update is the intuitive answer and the
wrong one, because it does not scale to a cache action's release history and
therefore will not happen. A check nobody performs provides no protection while
appearing to.

Verifying that a pin resolves to the tag it advertises is mechanical, runs
unattended, and catches the attack that pinning is meant to stop: a consumer
being pointed at a commit other than the release it believes it is running.
Depth of reading is reserved for what the diff would actually change — which is
a judgment a human makes, informed by what the job's token and secrets can reach,
now that both are bounded.

## What this does not do

- **It does not unpin anything or change any action version.** Pins stay pins;
  this checks them.
- **It does not block updates.** A verified pin update remains an ordinary review.
- **It does not remove Dependabot's Actions cadence**, which `dependency-maintenance`
  establishes and which this depends on to keep pins current.
- **It does not touch the deployment's behavior** — the same files reach the same
  host by the same protocol.
