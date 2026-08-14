# release-deploy.sh fails with "bad trap", and its primary update path is never installed

## Problem

Running `/opt/coterie/deploy/release-deploy.sh` on an existing install fails with
`trap: 26: bad trap` and deploys nothing.

Two defects chain to produce it.

**1. A bash script is executed with `sh`.** `release-deploy.sh` is `#!/bin/sh`,
and at line 53 it delegates:

```sh
exec sh "$cand" "$@"
```

where `$cand` is `deploy/update.sh` — which is `#!/usr/bin/env bash` and whose
line 26 is:

```bash
trap 'echo "[update.sh] ERROR on line $LINENO — exit $?" >&2' ERR
```

`ERR` is a bash pseudo-signal, not a POSIX one. Under `dash` — which is `/bin/sh`
on Debian, and this instance is Debian 13 — that is exactly `trap: 26: bad trap`.
Invoking with `sh` discards the shebang that says which interpreter the file
needs. Running the same file directly, as the operator did with a copy in `/tmp`,
works.

**2. The delegation target that should have been used is not installed.** The
`sh "$cand"` fallback is only reached because both preferred branches miss:

```sh
if [ -x "$INSTALL_DIR/coterie-provision" ]; then   # file does not exist
elif command -v coterie-provision >/dev/null 2>&1; # not on PATH
```

Confirmed on the production host: `/opt/coterie/coterie-provision` does not
exist and nothing named that is on `PATH`.

That is the more significant half. The script's own comment says the delegation
exists so that there is *"a single update code path"* — the hardened, testable
`coterie-provision update`. On this instance that path has never been reachable,
so every update has silently taken the bootstrap fallback instead, and the
fallback is the one that is broken. A design whose stated purpose is to have one
update path currently has one that is uninstalled and one that fails.

Nothing reports either condition. The script announces which branch it takes only
on the branches that work.

## Desired end state

- `release-deploy.sh` executes `update.sh` with an interpreter that satisfies its
  shebang, so a bash script is never run under `dash`.
- `coterie-provision` is present on an installed instance, so the delegation the
  design is built around is actually taken. If it is intentionally absent on some
  installs, the script says so when it falls back, rather than falling back
  silently.
- Falling back to the bootstrap path is visible in the output, since it means the
  hardened path was unavailable.

## Notes for whoever fixes this

- Prefer `exec "$cand" "$@"` (honouring the shebang, requiring the executable
  bit) over hardcoding `bash`, so the interpreter stays the script's own
  business. Confirm the executable bit survives whatever places the file —
  `deployment-updates` requires it for `*.sh`.
- Check the other direction too: `update.sh` is bash and uses at least one
  bashism. Either keep it bash and always invoke it as such, or make it POSIX and
  drop the `ERR` trap. Do not leave it bash-with-a-`sh`-caller.
- Establishing where `coterie-provision` is supposed to come from is part of this.
  The first-install path in `release-deploy.sh` places `coterie`, `seed`, and
  `create_admin`, and no path observed on the production host places
  `coterie-provision`.
- The production host was still running the 2026-07-09 copy of
  `release-deploy.sh` when this was found, because the fix that refreshes
  `deploy/` on update had not been deployed yet. Verify against the current
  script before concluding anything about which lines are live.

## Tasks

- [x] Reproduce: on a host without `coterie-provision`, run `release-deploy.sh`
  against an existing install and observe `bad trap`.
- [x] Fix the invocation so `update.sh` runs under the interpreter its shebang
  names.
- [x] Make the fallback announce itself, so taking the bootstrap path is visible
  rather than silent.
- [x] Determine what should install `coterie-provision` and ensure an installed
  instance has it, or state plainly in the script and the docs that its absence
  is expected and what the consequence is.
- [x] Add a test that `update.sh` is not invoked with an interpreter its shebang
  contradicts — the class of defect, not just this instance.
- [x] Verify the executable bit on `deploy/*.sh` survives placement by the update
  path.
