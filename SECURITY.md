# Security Policy

Coterie is the member-management system operated by **Industrious Kraken LLC**. 
We welcome good-faith security research under the terms below.

This program is run jointly: **The Neon Temple (NT Cyber Project Inc)** authorizes
testing of its live deployment; **Industrious Kraken LLC**, the maintainer,
authorizes testing of the open-source code and self-hosted instances and issues
CVEs. Both grant the safe harbor below for their respective assets.

## Supported versions

Only the latest tagged non-dev release is in scope (e.g. `v1.2.3`, not a `-dev`
or `-rc` build). Report against the newest deployed version; findings that only
affect an older tag are not eligible.

## Program type & authorization

This is a **private, invitation-only** program. Authorized testers are
**current members in good standing with The Neon Temple (NT Cyber Project
Inc)**, as determined by NT. Good standing and eligibility are at NT's sole
discretion and may be revoked at any time.

Before testing the **live deployment**, you must be on the tester roster: on the
Neon Temple Discord, message Rab/Charles or a member of the Protectorate to opt
in and acknowledge these rules. You will be issued a test account. We keep a
record of authorized testers — this is what your safe-harbor protection is tied
to. Self-hosters testing their **own** instance do not need to be on the roster.

## Scope

**In scope**

- The Coterie API and its web frontend on the live deployment (the host you
  were given when added to the roster).
- The `IndustriousKraken/coterie` source at the latest tagged release (format: v0.0.0).
- The `IndustriousKraken/theneontemple.com` signup/marketing frontend, and its
  public join/pay funnel where it drives the Coterie API on the live deployment.

**Out of scope**

- Third-party infrastructure and services: Stripe, Discord, DigitalOcean,
  Cloudflare, email providers. Report those to the respective vendor.
- The **Stripe-hosted Checkout page** — it is served by Stripe, not us. Card
  data never touches our servers (Stripe Checkout, PCI SAQ-A). Do not attempt to
  capture or submit real card data; use **Stripe test mode** only.
- The legacy WordPress site (retired).
- Any deployment or asset you were not explicitly authorized to test.
- **Not-yet-enabled integrations** — integrations not enabled on the live
  deployment, currently **Discord** and **UniFi** (UniFi is an unfinished stub).
  Their feature behavior isn't live, and incomplete behavior in a self-hosted
  build is not a vulnerability. *Exception:* a way to invoke, enable, or reach
  one of these **without authorization** is in scope — if the off switch doesn't
  hold, we want to know.

## Rewards

Rewards are paid for the **highest-tier outcome** you can demonstrate with a
working proof-of-concept. Report the full chain; we reward on impact, not on the
number of bugs in the chain.

| Outcome | Reward |
|---|---|
| Unauthorized disclosure of intended-private data via the frontend or API — by an unauthenticated actor, or by an authenticated user reaching data beyond their authorization | **$250** |
| …where the exposed data is **PII** | **$500** |
| A non-admin account gaining **admin control** or admin-only actions without authorization | **$500** |
| Accessing **another user's data** on a deployment where the **Member Directory** feature is *disabled* | **$500** |
| **Payment-integrity bypass**: obtaining paid membership or member benefits without valid payment, or manipulating charge/refund amounts | **$500** |

**PII** here means: legal name, email, postal address, phone number, payment
identifiers, government ID, or any custom member field an organization has
marked private (e.g. linked external-platform identities).

**Member Directory caveat:** on a deployment where the Member Directory feature
is *enabled*, reading member data exposed by that feature is intended behavior,
not a vulnerability. The horizontal-access reward applies only where the feature
is off and the data should not be reachable.

Not rewarded: access to data you are authorized to see, dependency CVEs without
a working PoC against our deployment, and anything in the *Out of scope* lists.

## Rules of engagement

- **Test in your own lab first.** Prefer a self-hosted instance for anything
  intrusive. On the live site, keep it non-destructive.
- **No availability impact.** No DoS, no volumetric/load testing, no automated
  scanners hammering the live site, no spam.
- **Stop at proof.** Do not exfiltrate, modify, delete, or persist data beyond
  the minimum needed to demonstrate the issue. To prove cross-account access,
  demonstrate it between **two test accounts you control** wherever possible.
- **Never touch real payment flows.** Stripe **test mode** only.
- **Do not use real members' data.** Do not use your own real member account to
  generate test noise; request a test account instead.

### Test accounts

Message us and we will create test accounts for the live deployment manually.
Self-hosters create their own. Do not attempt to self-register bulk accounts on
the live site.

## Data handling

If you incidentally access real personal data, **stop, do not retain or share
it, report it immediately, and delete any local copies** once we confirm
receipt. Handling of accessed personal data is judged as part of good faith.

## Safe harbor

We will not pursue or support legal action against authorized testers for
good-faith research that follows this policy. "Good faith" means: staying in
scope, following the rules of engagement, avoiding privacy violations and
service disruption, and giving us reasonable time to remediate before any
disclosure. If legal action is brought by a third party against someone who
followed this policy, we will make it known that their actions were authorized.

This authorization does not extend to actions that violate the law, and does not
waive third parties' rights (e.g. Stripe, Discord).

## Reporting

- **Preferred:** GitHub private vulnerability report —
  <https://github.com/IndustriousKraken/coterie/security/advisories/new>
- **Email:** bugs@industriouskraken.com
- Also announced in the Neon Temple Discord, but please file the report through
  one of the above so it is tracked.

Include: affected asset and version/commit, reproduction steps, a proof-of-concept,
and the impact you are claiming (which reward tier).

## Response & disclosure

- Acknowledgment within **3 business days**.
- We triage, confirm the tier, and agree on a fix timeline with you.
- **Coordinated disclosure:** please hold public disclosure until a fix ships or
  **90 days** pass, whichever comes first.
- **Duplicates:** only the first clear, reproducible report of a given issue is
  eligible for a reward. Multiple reports of the same underlying vulnerability —
  including variants, or different paths to the same root cause — count as one,
  and the earliest timestamped report is the one paid.
- **CVEs:** for confirmed vulnerabilities in the source, the maintainer
  (Industrious Kraken) will publish a GitHub Security Advisory and request a CVE
  where applicable, crediting you unless you ask otherwise.

## Acknowledgments

With thanks to the researchers who have helped keep Coterie safe:

<!-- Add credited reporters here (with permission). -->
