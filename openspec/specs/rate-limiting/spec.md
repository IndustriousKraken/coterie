# rate-limiting Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Money-moving endpoints are rate-limited per IP

The system SHALL apply a per-IP rate limit (`money_limiter`) of 10 requests per 60 seconds to money-moving endpoints and to public signup. Current callers:

- `POST /public/donate` — public donation flow.
- `POST /public/signup` — both signup modes (see below).
- `POST /public/events/:id/register` — public paid-event registration (unauthenticated, initiates a Stripe Checkout session).
- `POST /portal/api/payments/checkout`, `POST /portal/api/payments/charge-saved` — portal-initiated payments.
- `POST /portal/donate` API — logged-in donations.
- `POST /portal/admin/members/:id/record-payment` — admin manual payment recording.

Adding a money-moving endpoint without wiring `money_limiter` SHALL be treated as a defect. `/public/signup` SHALL subscribe to `money_limiter` in BOTH modes, applied BEFORE the bot-challenge provider so a bursting IP cannot burn the provider's quota. In payment mode the limiter caps card-testing on the Stripe-Checkout side-effect; in approval mode it caps unauthenticated mass account creation and verification-email amplification (each signup queues a verification email), which matters because the bot challenge defaults to disabled.

`POST /public/events/:id/register` SHALL follow the same before-the-provider ordering for the same reason, and additionally caps seat-squatting: each accepted request claims a seat against the event's capacity until its payment leaves `Pending`.

#### Scenario: Donation flood is rejected

- **WHEN** an IP submits 11 donation requests within 60 seconds
- **THEN** the 11th request SHALL be rejected by the rate limiter

#### Scenario: New money endpoint must subscribe to the limiter

- **WHEN** a new endpoint that records or initiates a payment is added
- **THEN** it SHALL invoke the shared `money_limiter` and be added to the rate-limited set; reviewers SHALL block PRs that omit this

#### Scenario: Payment-mode signup shares the money limiter

- **WHEN** the org's signup mode is `payment` and an IP at the money-limiter budget submits another signup
- **THEN** the request SHALL be rejected with `429` WITHOUT consulting the bot-challenge provider

#### Scenario: Approval-mode signup is also rate-limited

- **WHEN** the org's signup mode is `approval` and an IP at the money-limiter budget submits another signup
- **THEN** the request SHALL be rejected with `429` WITHOUT consulting the bot-challenge provider (the limiter applies regardless of signup mode)

#### Scenario: Public event registration is rate-limited before the provider is consulted

- **WHEN** an IP at the money-limiter budget submits another `POST /public/events/:id/register`
- **THEN** the request SHALL be rejected with `429` WITHOUT consulting the bot-challenge provider and WITHOUT claiming a seat

### Requirement: Rate-limiter mutex poisoning is recoverable

The in-memory rate limiter SHALL recover gracefully if its internal mutex becomes poisoned (e.g., due to a panic in another thread). The limiter SHALL log a warning and continue serving rather than propagating the panic.

#### Scenario: Poisoned mutex logs and recovers

- **WHEN** a thread panics while holding the rate-limiter mutex
- **THEN** subsequent calls SHALL log "RateLimiter mutex was poisoned; recovering" and continue best-effort

### Requirement: Periodic cleanup runs in a background task

The application SHALL spawn a background task per limiter to periodically purge expired buckets and prevent unbounded memory growth. The cadence SHALL match each limiter's window (login: ~15 min; money: ~1 min).

#### Scenario: Cleanup task runs continuously

- **WHEN** the application is running
- **THEN** background tasks SHALL invoke `limiter.cleanup()` on a regular cadence so the in-memory map does not grow without bound

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

