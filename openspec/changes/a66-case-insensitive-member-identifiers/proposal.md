# Change: Email and username match without regard to case

## Why

Two members in a month have been unable to sign in for the same reason, and
neither had done anything wrong.

- **`RelicIX`** (2026-09-12) typed his username in a different case. Five
  `unknown_email` denials, then the rate limiter — he saw "Too many login
  attempts" for a mistake the system never told him about.
- **`miarune`** (from 2026-09-02) is stored as `Mia@rupecreativehouse.com` and
  types `mia@rupecreativehouse.com`. Every attempt was `unknown_email`. Several
  password resets were tried; one completed successfully at 16:08 on 09-02, and
  the login thirteen seconds later still failed — the password was never the
  problem. Reset requests made with the lowercase address were also denied as
  unknown, silently, because the reset page must not reveal which addresses
  exist. She has not been able to sign in for four weeks.

Both lookups are byte-exact: `find_by_email` is `WHERE email = ?` and
`find_by_username` is `WHERE username = ?`, with no case folding on the query or
the column. Eight members currently have an email containing capitals; each can
sign in or reset only by reproducing that capitalisation exactly, which phones
and mail clients routinely "correct" away from.

The `UNIQUE` constraints are byte-exact too, so nothing stops a new signup
`relicix` from existing beside `RelicIX`. That is harmless only while login is
case-sensitive — and it is the reason login cannot simply be made
case-insensitive on its own.

## What Changes

Three separate properties, deliberately kept apart:

- **Matching is case-insensitive.** Every lookup of a member by email or username
  — login on both surfaces, password reset, signup's duplicate path, public
  donation attachment, bulk import — ignores case. Implemented once, in the
  repository, so no caller can do it differently.
- **Storage and display are unchanged.** `RelicIX` is still shown as `RelicIX`.
  Nothing is lowercased on write, and no existing row is rewritten.
- **Uniqueness is case-insensitive.** A new email or username that differs from
  an existing one only by case is refused, through the same unique-violation path
  signup and import already use. This is what makes case-insensitive matching
  unambiguous: there can never be two members answering to one login.

Production was checked on 2026-09-30: no two usernames and no two emails differ
only by case, so the uniqueness constraint applies to the existing data without
any reconciliation. The eight mixed-case members are fixed by the matching change
the moment it deploys — with no data migration, and without breaking anyone's
saved password-manager entry, which rewriting their stored address would have.

## Consequence worth naming

Public donations already attach to an existing member when the donor's email
matches. After this change that match also ignores case. That is the existing
rule applied consistently, not a new one — today a donation from
`mia@rupecreativehouse.com` is recorded as anonymous while one from `Mia@…`
attaches, which is not a distinction anyone intended.

## What this does not do

- It does not add Unicode case folding. Case-insensitivity is ASCII, matching
  SQLite's `NOCASE` and `lower()`. Every identifier in production is ASCII.
- It does not change which statuses may sign in, the error messages shown, or the
  rate limiter's budgets.
- It does not normalise or rewrite stored identifiers.
