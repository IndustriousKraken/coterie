#!/usr/bin/env python3
"""Verify that every action pin in .github/workflows/ is what it claims to be.

A pin is `uses: owner/repo@<sha>  # <claim>`. GitHub resolves the SHA and never
reads the comment, so a pin reading `# v3.0.2` while pointing at an unrelated
commit is indistinguishable from a correct one by eye — and that is the shape of
a supply-chain attack against a consumer who has already pinned.

Four things are checked:

  1. The reference is a full 40-hex commit SHA, not a tag or branch.
  2. The comment names something. An unverifiable pin is not a verified one.
  3. The claim resolves upstream and agrees with the SHA:
       - a TAG claim must EQUAL the tag's commit (annotated tags are
         dereferenced first — `git/ref/tags/<tag>` yields a tag object, and
         comparing without following it reports every correct pin as a
         mismatch);
       - a MOVING claim (a branch, or a tag upstream republishes) must only
         EXIST upstream. Equality would fail on every upstream push and pressure
         the pin to chase the branch, which is the tracking pinning exists to
         avoid; ancestry fails too, because an upstream that rebases leaves a
         legitimately pinned commit unreachable from the branch while the commit
         itself is still there. `dtolnay/rust-toolchain` does exactly this.
  4. No action appears at two different SHAs across the workflows. Each pin can
     match its own comment while one job silently runs an older action.

Comment grammar: the first whitespace-separated token after `#` is the claimed
ref, and the SECOND must be exactly "branch" or "moving" to mark it as a moving
reference — `# stable branch — moving ref, pinned 2026-07-09`. Anything else is
a tag claim, which is the stronger assertion and the default. The marker is
positional rather than a search of the comment text so that prose which happens
to mention a branch (`# v1.2.3 fixes branch handling`) cannot silently downgrade
a pin from equality to existence-only.

The transport is injectable: `--refs FILE` reads a JSON fixture of API path ->
response instead of calling GitHub, so the tests run offline against the same
resolution logic (dereferencing included) that CI runs live. An absent path is a
404. Live lookups honour $GITHUB_TOKEN.

    {"/repos/Swatinem/rust-cache/git/ref/tags/v2.9.2":
        {"object": {"type": "tag", "sha": "<tag object sha>"}},
     "/repos/Swatinem/rust-cache/git/tags/<tag object sha>":
        {"object": {"sha": "<commit sha>"}}}
"""

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import NamedTuple, Optional

USES = re.compile(r"^\s*(?:-\s+)?uses:\s*(?P<ref>\S+)\s*(?:#\s*(?P<comment>.*?))?\s*$")
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
MOVING_MARKERS = ("branch", "moving")

API = "https://api.github.com"


class Pin(NamedTuple):
    path: Path
    line: int
    ref: str  # the whole `owner/repo@sha`, as written
    action: str  # `owner/repo`
    sha: str  # whatever followed the `@`
    claim: Optional[str]  # first token of the comment, if any
    moving: bool

    def where(self) -> str:
        return f"{self.path}:{self.line}"


def live_fetch(path: str):
    """GET an API path. None means 404 — absent upstream."""
    req = urllib.request.Request(
        f"{API}{path}",
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "coterie-verify-action-pins",
        },
    )
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as err:
        if err.code == 404:
            return None
        raise


def upstream_repo(action: str) -> str:
    """`owner/repo` — an action may live in a subdirectory of its repository,
    and `/repos/owner/repo/subdir/...` is not an API path."""
    return "/".join(action.split("/")[:2])


class Resolver:
    """One request per lightweight tag, two for an annotated one."""

    def __init__(self, fetch=live_fetch):
        self.fetch = fetch

    def tag_commit(self, action: str, tag: str) -> Optional[str]:
        action = upstream_repo(action)
        ref = self.fetch(f"/repos/{action}/git/ref/tags/{tag}")
        if ref is None:
            return None
        obj = ref["object"]
        if obj["type"] != "tag":  # lightweight tag: already a commit
            return obj["sha"]
        # Annotated: `git/ref/tags/<tag>` gave the tag OBJECT, not the commit.
        # Comparing here would report every correct pin as a mismatch.
        annotated = self.fetch(f"/repos/{action}/git/tags/{obj['sha']}")
        return None if annotated is None else annotated["object"]["sha"]

    def commit_exists(self, action: str, sha: str) -> bool:
        return self.fetch(f"/repos/{upstream_repo(action)}/git/commits/{sha}") is not None


def collect_pins(workflows: Path) -> list[Pin]:
    pins = []
    for path in sorted(workflows.glob("*.y*ml")):
        for number, text in enumerate(path.read_text().splitlines(), start=1):
            match = USES.match(text)
            if not match:
                continue
            ref = match.group("ref")
            # Local composite actions and container images are not pins.
            if ref.startswith("./") or ref.startswith("docker://"):
                continue
            action, _, sha = ref.partition("@")
            comment = match.group("comment") or ""
            tokens = comment.split()
            pins.append(
                Pin(
                    path=path,
                    line=number,
                    ref=ref,
                    action=action,
                    sha=sha,
                    claim=tokens[0] if tokens else None,
                    moving=len(tokens) > 1 and tokens[1].lower() in MOVING_MARKERS,
                )
            )
    return pins


def verify(pins: list[Pin], resolver) -> list[str]:
    failures = []

    for pin in pins:
        if not FULL_SHA.match(pin.sha):
            failures.append(
                f"{pin.where()}: {pin.ref} is not pinned to a full 40-character "
                f"commit SHA — a tag or branch reference is what pinning prevents"
            )
            continue
        if pin.claim is None:
            failures.append(
                f"{pin.where()}: {pin.ref} has no version comment — an "
                f"unverifiable pin is not a verified one"
            )
            continue

        if pin.moving:
            if not resolver.commit_exists(pin.action, pin.sha):
                failures.append(
                    f"{pin.where()}: {pin.ref} claims moving reference "
                    f"'{pin.claim}', but that commit is not in {pin.action}"
                )
            continue

        upstream = resolver.tag_commit(pin.action, pin.claim)
        if upstream is None:
            failures.append(
                f"{pin.where()}: {pin.ref} claims tag '{pin.claim}', which does "
                f"not exist in {pin.action}"
            )
        elif upstream != pin.sha:
            failures.append(
                f"{pin.where()}: {pin.ref} claims tag '{pin.claim}', which is "
                f"{upstream} upstream"
            )

    by_action: dict[str, dict[str, list[Pin]]] = {}
    for pin in pins:
        by_action.setdefault(pin.action, {}).setdefault(pin.sha, []).append(pin)
    for action, shas in sorted(by_action.items()):
        if len(shas) > 1:
            where = "; ".join(
                f"{pin.where()} {pin.ref} (# {pin.claim})"
                for pins_at in shas.values()
                for pin in pins_at
            )
            failures.append(
                f"{action} is pinned at {len(shas)} different commits across the "
                f"workflows: {where}"
            )

    return failures


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workflows", type=Path, default=Path(".github/workflows"))
    parser.add_argument(
        "--refs",
        type=Path,
        help="JSON fixture of API path -> response, used instead of the GitHub API",
    )
    args = parser.parse_args(argv)

    if args.refs:
        canned = json.loads(args.refs.read_text())
        resolver = Resolver(fetch=canned.get)
    else:
        resolver = Resolver()

    pins = collect_pins(args.workflows)
    if not pins:
        print(f"no `uses:` pins found under {args.workflows}", file=sys.stderr)
        return 1

    failures = verify(pins, resolver)
    for failure in failures:
        print(failure, file=sys.stderr)
    if failures:
        print(f"{len(failures)} pin problem(s)", file=sys.stderr)
        return 1

    print(f"{len(pins)} action pins verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
