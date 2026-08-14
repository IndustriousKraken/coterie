# Change: Clear the outstanding advisories, mostly by deleting unused dependencies

## Why

The advisory check added by `dependency-maintenance` found seven vulnerabilities
on its first run. Two are already fixed — `lettre` is at 0.11.22 and
`quinn-proto` at 0.11.16. Five remain, and until they are resolved the check
fails on every pull request.

That is the real cost. A check that is always red stops being read. Everyone
learns that the audit job is "just always failing," and the next genuine advisory
arrives into a signal nobody looks at — which is precisely the state
`dependency-maintenance` was written to end.

What remains:

| Crate | Advisory | Reachable here? |
|---|---|---|
| `ammonia` 4.1.3 | XSS via SVG `animate`/`set` | No — the sanitizer's tag allow-list is a fixed set that contains no SVG element |
| `rsa` 0.9.10 | Marvin timing attack, **no fixed version exists** | No — pulled only by `sqlx-mysql`, a driver this application cannot use |
| `rustls-webpki` 0.101.7 ×3 | name-constraint handling | Old TLS stack, reached only through `async-stripe`'s transitive `rustls` 0.21 |

Note what that column says: **none of the five is exploitable in this
application.** They are all real advisories against code that is genuinely
present, and none of them is a live vulnerability here. That is the normal case
for a dependency audit, and it is exactly why "just fix the critical ones" is the
wrong instinct — severity ranked these almost backwards, with the 9.1 `lettre`
finding being inapplicable (wrong TLS backend) while the one guarding public HTML
sat unremarked.

## What Changes

- **`ammonia` moves to a fixed version.** Not reachable through the current
  allow-list, but this crate is the boundary that makes member-authored Markdown
  safe to serve publicly, and "the configuration happens to exclude the
  vulnerable element" is a weaker guarantee than not having the bug.

- **The unused database drivers are removed, which deletes `rsa` entirely.**
  `sqlx` is declared without `default-features = false`, so its default feature
  set pulls every driver — MySQL and Postgres included — into an application that
  is SQLite-only. `rsa` arrives through `sqlx-mysql`.

  The advisory says *"No fixed upgrade is available!"*, which reads like a forced
  waiver. It is not. The crate is here because of a feature nobody asked for, and
  the fix is to stop compiling a MySQL driver this application can never call.

- **`async-stripe` moves to a version that does not carry the old TLS stack**, or,
  if none exists, the finding is waived explicitly. `rustls-webpki` 0.101.7
  arrives via `async-stripe` → `hyper-rustls` 0.24 → `rustls` 0.21, while the
  application's own TLS is `rustls` 0.23. Two TLS stacks are linked; only one is
  used.

- **Anything that genuinely cannot be fixed gets a recorded waiver**, naming the
  advisory, the reason it is not reachable, and a date to look again — not a
  blanket ignore.

## Why removal beats waiving

Three of the five findings are in code paths this application does not use, and
two of those exist only because a dependency's default features were never
narrowed. Waiving them would leave the code compiled in, still linked, still
carrying whatever is found in it next — and would grow an ignore list that is
itself never re-read.

Removing an unused driver is smaller, permanent, and reduces build time. A waiver
is what remains when removal genuinely is not possible.

## What this does not do

- **It does not change application behavior.** No database, TLS, or sanitizer
  behavior changes; the sanitizer's allow-list is untouched.
- **It does not relax the advisory check** or add an ignore mechanism for
  convenience. A waiver, if one proves necessary, is a specific decision about a
  specific advisory.
- **It does not adopt a blanket "upgrade everything" pass.** The routine cadence
  established by `dependency-maintenance` handles that.
