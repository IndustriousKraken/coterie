# Change: Credential rate limiting counts failures, and keys on the account

## Why

On 2026-08-13 an admin locked themselves out of the production instance with
**five consecutive successful logins.** Not one failed password was involved:

```
15:03:41  auth.login  ok      ← admin, first factor
15:03:41  auth.totp   ok      ← their second factor
15:05:19  auth.login  ok
15:06:46  auth.login  ok      ← a different account
15:06:46  auth.login  ok      ← the same submit, twice
15:07:20  auth.rate_limited   ← denied
```

`RateLimiter::check_and_record` records every attempt before authenticating and
never clears on success — there is no reset path in the file. The budget is 5 per
15 minutes per IP.

Three things compound:

1. **Success costs the same as failure.** Doing everything right consumes the
   budget for defending against people doing it wrong.
2. **Correct MFA costs double.** The limiter's map is keyed by `IpAddr` alone —
   the `endpoint` argument is used only in the log line — so a second factor
   spends a slot. Logging in *more securely* exhausts the budget faster.
3. **A double-submit costs double.** Two of those entries are one login.

And the per-IP key has a worse consequence than the one that triggered this. Ten
members at an event venue share one NAT address. Under a per-IP budget, a handful
of mistyped passwords among them locks out everyone at the venue — including
people who have not attempted anything. The budget meant to protect accounts is
spent by whoever happens to share an address, and the failure lands on people
with no way to understand it.

This has a precedent in this repository. `rate-limiting` already records a
production incident on 2026-07-29 where coupling recovery to the login budget
locked a member out of the mechanism built for forgotten passwords, and canon was
amended to separate them. The same shape recurs here: a budget shared by parties
that should not share it.

## What Changes

- **Only failed attempts count.** A successful authentication consumes nothing.
  This is what the budget is for — brute force is a volume of *failures*, and a
  success is the outcome the limiter exists to make expensive to guess, not to
  ration.

  This alone resolves the double-submit case, because both submissions succeeded.
  No de-duplication logic is needed and none is added.

- **The primary budget keys on the account being attempted, not the address.**
  Brute force targets an account; that is where a tight budget belongs. Ten
  people at one venue no longer contend for one allowance, and one member's
  fumbling cannot lock out the room.

- **A second per-address budget becomes an explicit sustained-abuse path**, not a
  bigger copy of the first. It is evaluated on the **breadth of accounts a source
  is failing against** — how many distinct identifiers, including ones matching no
  member — rather than on raw failure count, because breadth is what separates the
  two populations. A room of members produces failures concentrated on a few
  accounts by people who roughly know their own passwords; stuffing produces
  failures spread thin across many accounts, a large share of them names that do
  not exist. A raw count cannot tell those apart.

  One member locking themselves out is ordinary and stays a private matter between
  that member and their own account: it SHALL NOT meaningfully advance the address
  budget.

- **An attempt is denied when either budget is exhausted**, and the denial looks
  the same either way, so the response does not become an oracle for which
  accounts exist.

- **Second-factor failures count against the account**, preserving exactly the
  threat canon already names — a stolen-password attacker brute-forcing a 6-digit
  code — and keying it to the account under attack rather than to whatever
  address the attacker happens to use.

## The tradeoff this accepts

A per-account budget means an attacker who knows a member's username can spend
that member's failure budget and deny them login for the window. That is a real
cost, and it is smaller than what it replaces: today the same attacker denies
login to *everyone sharing an address with the victim*, which at an event is the
entire room.

Two things bound it. The window is short and self-recovering. And recovery
already has its own separate budget under existing canon, so a member locked out
of login can still request a reset — the escape hatch the 2026-07-29 amendment
exists to keep open.

## What this does not do

- **It does not raise or lower the failure threshold.** Five failures against one
  account in fifteen minutes remains the limit; only what counts toward it, and
  what it is keyed to, change.
- **It does not give the second factor a fresh allowance.** Canon's reasoning for
  refusing that stands; this change re-keys the shared budget rather than
  splitting it.
- **It does not change the recovery limiter or the money-endpoint limiter.**
- **It does not add lockout notification or an admin unlock control.** Both are
  reasonable and neither is needed to stop the reported defect.
