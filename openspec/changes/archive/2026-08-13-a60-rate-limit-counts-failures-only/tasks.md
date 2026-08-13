# Tasks

## 1. Split check from record

- [x] 1.1 `src/api/state.rs`: replace `check_and_record` with a read-only check
  and a separate record-failure call. The check must still run **before**
  credential verification so an over-limit attempt does no password hashing —
  that ordering is the reason the combined call existed and it must survive the
  split.
- [x] 1.2 Record only after verification fails. A successful authentication
  records nothing.
- [x] 1.3 Delete no logging: the `auth.rate_limited` event on rejection is
  required by `auth-logging` and stays exactly as it is.
- [x] 1.4 Do not add de-duplication of repeated submissions. Once successes cost
  nothing, a double-submitted successful login costs nothing, and dedup logic
  would be machinery for a problem that no longer exists.

## 2. Per-account budget

- [x] 2.1 Key the tight budget on the submitted identifier, at the existing 5 per
  15 minutes. Key on it **as submitted** — normalized consistently, but without
  requiring it to resolve to a member — so that consuming budget cannot reveal
  which accounts exist.
- [x] 2.2 Normalize the key the same way login resolves identifiers (case,
  trimming), or an attacker gets a fresh budget per capitalization.
- [x] 2.3 Second-factor failures count against the same account budget. This
  preserves the threat canon names — post-password brute force of a 6-digit code
  — while keying it to the account under attack.

## 3. Per-address sustained-abuse path

- [x] 3.1 Track failures per address by **breadth**: the number of distinct
  identifiers failed against within the window, not raw failure count. Breadth is
  what distinguishes a venue from stuffing; a raw count cannot.
- [x] 3.2 Failures against identifiers matching no member count toward this
  signal — a source guessing at nonexistent accounts is not a member mistyping.
- [x] 3.3 Keep it internal. The rejection an over-limit caller sees must be
  identical whether the identifier exists or not, or this becomes the existence
  oracle the requirement forbids.
- [x] 3.4 Choose the allowance so that one member exhausting their own account
  budget does not meaningfully advance the address budget, and record the
  reasoning in a comment with the venue case named. A number without its reasoning
  is a number the next person will change blindly.

## 4. Denial

- [x] 4.1 Reject when either budget is exhausted, before verification.
- [x] 4.2 The two rejections are indistinguishable to the caller — same status,
  same body, same timing characteristics as far as is practical.

## 5. Tests

- [x] 5.1 Many successful logins in quick succession — including a correct second
  factor and the same form submitted twice — consume no budget and are never
  rejected. This is the reported defect; assert it directly.
- [x] 5.2 Six failures against one account are rejected on the sixth.
- [x] 5.3 A member who exhausts their own account budget does not prevent a
  different member from signing in successfully **from the same address**. This is
  the venue case and the reason the key moved.
- [x] 5.4 Several members at one address each failing a few times, one of them
  exhausting their own budget, does not reach the address budget.
- [x] 5.5 One address failing across many distinct identifiers is rejected even
  though no single account reached its limit.
- [x] 5.6 Failures against nonexistent identifiers count toward the address
  signal, and the responses do not differ from those for existing accounts.
- [x] 5.7 Identifier normalization: varying capitalization does not yield a fresh
  budget.
- [x] 5.8 Recovery remains on its own budget — an account locked out of login can
  still request a reset. This is existing canon and must not regress.
- [x] 5.9 An over-limit attempt performs no password verification. Assert it,
  because the cost of losing that ordering is a hashing DoS and it is invisible
  otherwise.
