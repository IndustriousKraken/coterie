# rate-limiting Specification Delta

## RENAMED Requirements

- FROM: `### Requirement: Credential flows are rate-limited per IP`
- TO: `### Requirement: Credential flows are rate-limited on failed attempts, keyed by account and by address`

## MODIFIED Requirements

### Requirement: Credential flows are rate-limited on failed attempts, keyed by account and by address

The system SHALL rate-limit credential-handling endpoints on **failed** attempts only. A successful authentication SHALL consume no budget. The current callers are:

- `POST /auth/login` (api handler) — first-factor password attempt.
- `POST /login` (web-template handler) — first-factor password attempt.
- `POST /auth/login/totp` (api handler) — second-factor TOTP/recovery-code attempt.
- `POST /login/totp` (web handler) — second-factor TOTP/recovery-code attempt.

Counting successes rations the outcome the limiter exists to protect rather than the guessing it exists to stop. On 2026-08-13 an administrator was locked out of the production instance by five consecutive **successful** logins, two of which were one double-submitted form and one of which was a correct second factor — using multi-factor authentication correctly consumed the budget twice as fast as not using it.

Because a success costs nothing, a form submitted twice costs nothing when it succeeds. The system SHALL NOT add de-duplication of repeated submissions; the counting rule makes it unnecessary.

The primary budget SHALL be keyed by the **account** an attempt names, at 5 failures per 15 minutes. Brute force targets an account, and that is where a tight budget belongs. The budget SHALL be keyed by the identifier as submitted, whether or not it resolves to an existing member, so that consuming budget does not reveal which accounts exist.

A second budget SHALL be keyed by **client address** as a distinct, deliberately hard-to-reach path for sustained abuse, not as a second copy of the per-account rule with a bigger number. It exists to catch credential stuffing and to bound the password-verification work a single source can compel, which matters because verification is deliberately expensive.

It SHALL be evaluated on the **breadth of accounts a source is failing against** — the number of distinct identifiers attempted unsuccessfully — and not on raw failure count alone. Breadth is what separates the two populations. A room of members signing in produces failures concentrated on a few accounts, each belonging to someone who knows roughly what their password is. Credential stuffing produces failures spread across many accounts with one or two tries each, a large share of them identifiers that match no member at all. A raw count cannot tell those apart; breadth can.

Failures against identifiers that match no member SHALL count toward this signal, because a source guessing at accounts that do not exist is not plausibly a member mistyping their own password. This SHALL remain an internal signal only: it SHALL NOT change what the caller observes, since a rejection that differed for unknown accounts would be the existence oracle this requirement otherwise forbids.

The allowance SHALL be set so that a shared address carrying many legitimate members, several of whom mistype, does not reach it. One member exhausting their own account budget SHALL NOT meaningfully advance the address budget; locking oneself out is ordinary, and it SHALL remain a private event between a member and their own account.

Keying the tight budget to the address is what this replaces, and the reason is that addresses are shared. Ten members at an event venue present one address; under a per-address budget a few mistyped passwords among them deny login to everyone there, including members who have attempted nothing. This repeats the failure already recorded in this capability for 2026-07-29, where a budget shared between login and recovery locked a member out of the mechanism built for forgotten passwords: a budget shared by parties that should not share it.

An attempt SHALL be rejected when **either** budget is exhausted, and the rejection SHALL be indistinguishable between the two cases, so the response does not reveal which accounts exist or which are under attack.

Both budgets SHALL be consulted **before** credentials are verified, so an attempt that is already over the limit does no verification work.

The limiters SHALL be stored on `AppState` and shared across all surfaces so the same caller cannot get a fresh budget by hitting parallel paths. An attacker who exhausts an account's budget on `/auth/login` cannot switch to `/auth/login/totp` (or vice versa) to get a fresh allowance.

Second-factor failures SHALL count against the account's budget, preserving the protection against a stolen-password attacker brute-forcing the 6-digit TOTP code space while keying it to the account under attack rather than to the address the attacker happens to be using.

This design accepts a cost: an attacker who knows a member's username can spend that member's failure budget and deny them login for the window. That is smaller than what it replaces, where the same attacker denies login to every member sharing an address with the victim. The window is short and self-recovering, and recovery keeps its own budget below, so a member denied login can still request a reset.

**Password recovery SHALL NOT share these budgets.** `POST /forgot-password` SHALL be limited by a separate `recovery_limiter` with its own per-IP bucket, so exhausting the credential budget does not also close the recovery path.

The reason is that the budgets protect against different things and coupling them produces a trap: the user most likely to need a password reset is the user who has just failed several logins, and under a shared budget that is exactly the user who can no longer request one. Forgetting a password then locks the member out of the mechanism built for forgotten passwords, and the only escape is to wait out a window nothing tells them about. This is not hypothetical — a member hit it in production on 2026-07-29, exhausting the budget on five failed logins and then receiving `429` on `/forgot-password`.

Recovery SHALL still be limited, because it sends email and is therefore abusable as an amplification vector; it simply SHALL be limited independently.

#### Scenario: Sixth failed attempt against one account is rejected

- **WHEN** the same account is the subject of 6 failed login attempts within a 15-minute window
- **THEN** the 6th attempt SHALL be rejected with `429 Too Many Requests`

#### Scenario: Successful logins consume no budget

- **WHEN** a member logs in successfully many times in quick succession, including a correct second factor and a double-submitted form
- **THEN** no attempt SHALL be rejected, and no budget SHALL have been consumed

#### Scenario: One member's failures do not lock out others on the same address

- **WHEN** a member exhausts their account's failure budget from an address shared with other members
- **THEN** another member signing in successfully from that same address SHALL NOT be rejected

#### Scenario: Many accounts failing from one address is still caught

- **WHEN** one address produces failed attempts across many distinct accounts beyond the per-address allowance
- **THEN** further attempts from that address SHALL be rejected even though no single account reached its own limit

#### Scenario: A venue full of members fumbling does not trip the address budget

- **WHEN** several members at one shared address each fail a few times against their own accounts, one of them exhausting their own account budget
- **THEN** the address budget SHALL NOT be reached, and every other member at that address SHALL still be able to sign in

#### Scenario: Guessing at accounts that do not exist counts toward the address signal

- **WHEN** one address produces failed attempts against a succession of identifiers that match no member
- **THEN** those attempts SHALL count toward the address budget, and the caller SHALL NOT be able to tell from the responses which identifiers existed

#### Scenario: A rejection does not reveal whether the account exists

- **WHEN** attempts are rejected for an identifier that matches no member and for one that does
- **THEN** the two rejections SHALL be indistinguishable to the caller

#### Scenario: Reset does NOT share the budget with login

- **WHEN** an account exhausts the login limit and the member then immediately attempts a password-reset request
- **THEN** the reset SHALL be evaluated against `recovery_limiter` and SHALL be accepted if that separate budget has room; a member locked out of login SHALL still be able to request a reset

#### Scenario: Limit resets after the window

- **WHEN** 15 minutes pass after a budget is exhausted
- **THEN** subsequent attempts SHALL be accepted again up to the budget

#### Scenario: TOTP second-factor failures share the account's budget

- **WHEN** 5 wrong TOTP codes are submitted for an account to `/auth/login/totp` (or `/login/totp`) within a 15-minute window
- **THEN** the 6th submission SHALL be rejected with `429 Too Many Requests`; this prevents a stolen-password attacker from brute-forcing the 6-digit TOTP code space

#### Scenario: Switching surfaces does not multiply the budget

- **WHEN** an account's budget is exhausted by 5 failed password attempts on `/auth/login` and a `POST /auth/login/totp` is then attempted for that account
- **THEN** the TOTP attempt SHALL ALSO be rejected with `429`; the shared budget does not give a fresh allowance for the second-factor endpoint

#### Scenario: Recovery is still limited on its own bucket

- **WHEN** an IP submits password-reset requests beyond the `recovery_limiter` budget
- **THEN** the excess requests SHALL be rejected with `429`; separating the budget SHALL NOT mean leaving recovery unlimited, because each request sends email
