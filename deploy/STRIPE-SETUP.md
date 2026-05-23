# Stripe Setup

Step-by-step guide for wiring Coterie to a Stripe account. Covers
API keys, webhook endpoint registration, the events to subscribe
to, and the API version to pin.

This is the missing piece between "I've got the binary running"
and "members can actually pay dues." Allow ~15 minutes the first
time.

---

## 0. Test mode vs. live mode

Stripe has two completely separate environments:

- **Test mode** — fake money, fake cards, separate API keys, separate
  webhook endpoints. Use this while verifying the deploy works.
- **Live mode** — real money. Switch to this once test mode confirms
  everything is wired up.

The Stripe dashboard has a toggle (usually top-right or in the
nav) labeled "Test mode." It changes which keys you see and which
webhooks you're managing. **The API keys, signing secrets, and
webhook endpoints are NOT shared between modes** — you'll register
each independently.

> **Both modes need their own webhook configured** if both will be
> used (even sequentially — e.g. test mode now, live mode later via
> the switchover). Live charges sent without a live-mode webhook
> registered will succeed at Stripe but Coterie will never hear
> about them; dues won't extend, payments stay Pending.

Workflow:

1. Set up test-mode keys + webhook → verify a fake test charge
   flows through the system end-to-end.
2. Switch to live mode → repeat the same registration steps with
   live-mode keys.
3. Update `.env` with live-mode values when you're ready to take
   real money.

The rest of this doc applies to either mode.

> **Tip — verify before going live.** If this is a fresh deploy and
> you want to confirm Stripe is wired correctly before any real money
> moves, use the **test-mode-first workflow** described in section 8
> below. The `coterie-provision` wizard supports it as a one-line
> option, and a single `switch-stripe-to-live` subcommand transitions
> the box to live mode when you're done verifying.

---

## 1. Grab the API keys

Stripe Dashboard → **Developers → API keys** (or sometimes the
left nav has "API keys" directly).

You need two:

- **Publishable key** — starts with `pk_test_…` or `pk_live_…`.
  Safe to embed in frontend code; identifies your account but
  can't make charges by itself.
- **Secret key** — starts with `sk_test_…` or `sk_live_…`. Never
  commit this; never expose in frontend. Coterie uses it
  server-side to talk to Stripe's REST API.

Copy both. They go in `.env`:

```
COTERIE__STRIPE__ENABLED=true
COTERIE__STRIPE__PUBLISHABLE_KEY=pk_test_...
COTERIE__STRIPE__SECRET_KEY=sk_test_...
```

---

## 2. Register the webhook endpoint

Stripe needs to POST to Coterie whenever something happens on
Stripe's side (charge succeeded, refund processed, subscription
cancelled). This is the webhook endpoint.

Stripe Dashboard → **Developers → Webhooks** (or just "Webhooks"
in the nav). Click **Add endpoint**.

### Endpoint URL

```
https://<your-domain>/api/payments/webhook/stripe
```

For Neon Temple: `https://coterie.neontemple.com/api/payments/webhook/stripe`.

Notes:
- Must be HTTPS in live mode. (Stripe allows plain HTTP in test
  mode but Caddy gives you HTTPS for free anyway.)
- Stripe does NOT verify the URL is reachable at registration
  time. You can register before DNS resolves; webhooks pile up
  in "Pending" and retry for ~3 days.
- The path is exact and case-sensitive.

### Event destination scope

Pick **"Your account."**

The alternative ("Connected accounts") is for Stripe Connect
platforms processing payments on behalf of *other* businesses —
not Coterie's model.

### API version

Pick **2020-08-27** (likely shown as just "2020" in the
dropdown). This is the version `async-stripe 0.39` was built
against; newer API versions can change payload shapes and break
deserialization in subtle ways.

When `async-stripe` upgrades to a newer Stripe API in a future
Coterie release, this version pin should bump in lock-step.

### Events to subscribe to

Coterie's webhook dispatcher handles these nine event types. Select
exactly these — other events Stripe might send get logged and
ignored by Coterie, but they add noise.

| Event | What Coterie does with it |
| ----- | -------------------------- |
| `checkout.session.completed` | Flips a Pending payment to Completed, extends dues, schedules next renewal |
| `checkout.session.expired` | Marks the Pending payment Failed |
| `payment_intent.succeeded` | Idempotency-safe dues extension for direct saved-card charges |
| `payment_intent.payment_failed` | Marks the Pending row Failed |
| `charge.refunded` | Mirrors an out-of-band (Stripe-dashboard) refund to Coterie's Payment row |
| `invoice.paid` | Stripe-managed subscription renewed — extends dues |
| `invoice.payment_failed` | Notifies member + dispatches admin alert |
| `customer.subscription.deleted` | Flips a stripe_subscription member to manual billing |
| `customer.subscription.updated` | Observed; no action by default (logged) |

In Stripe's UI these are grouped under **Checkout**,
**PaymentIntent**, **Charge**, **Invoice**, and **Customer**
categories. Use "Select all events" if you'd rather not pick
individually — Coterie just ignores the ones it doesn't react to.

### Save the endpoint

Click **Add endpoint**. Stripe creates it and routes you to the
endpoint's detail page.

---

## 3. Grab the webhook signing secret

On the endpoint detail page, find the **"Signing secret"** section.
Click **Reveal** (or the eye icon). You'll see something like:

```
whsec_AbCd1234EfGh5678IjKl...
```

This is what Coterie uses to verify webhook payloads actually came
from Stripe (HMAC signature check inside
`src/payments/webhook_dispatcher.rs`). Anyone who can guess this
secret can forge events to Coterie — keep it as carefully as the
secret key.

Copy it into `.env`:

```
COTERIE__STRIPE__WEBHOOK_SECRET=whsec_AbCd1234...
```

---

## 4. Restart Coterie

After updating `.env`:

```bash
sudo systemctl restart coterie
sudo journalctl -u coterie -f
```

Watch for any startup errors. Coterie should log something like:

```
Stripe client initialized
```

near the end of startup. If it logs an error about Stripe config,
fix `.env` and restart again.

---

## 5. Send a test event

Back in the Stripe dashboard, on the endpoint's detail page, click
**"Send test webhook"** (sometimes "Send test event"). Pick
`payment_intent.succeeded` — it's a common one Coterie handles.

In another terminal on the droplet:

```bash
journalctl -u coterie -f
```

You should see something like:

```
Webhook event received: payment_intent.succeeded
No matching local Payment for payment_intent pi_test_…
```

The "no matching local Payment" is fine — the test event uses
synthetic IDs that don't correspond to real Coterie rows. What
matters is:

1. Coterie received the webhook.
2. The signature verified (no "Invalid signature" error).
3. The event deserialized cleanly (no serde errors).

The Stripe dashboard's test-event UI also shows a response — should
be `200 OK`. If it's anything else (especially 401 from CSRF, 500
from a panic), check Coterie's logs.

---

## 6. Real-world verification (optional but recommended)

In **test mode**, walk through a real-ish payment flow:

1. Set Coterie's signup or donate flow to a small amount.
2. Use Stripe's test card `4242 4242 4242 4242` with any future
   expiry, any 3-digit CVC, any ZIP.
3. Confirm:
   - Stripe dashboard shows the charge in test mode
   - Coterie's Payment row shows Completed
   - The webhook event arrived in Coterie's logs
   - The member's `dues_paid_until` advanced (if applicable)

If all four are green, the wiring is correct.

Then repeat the registration steps in **live mode** with live-mode
keys + a separate webhook endpoint, and update `.env` for
production.

---

## 7. Troubleshooting

**`Invalid signature`** — webhook secret in `.env` doesn't match
the one Stripe is signing with. Most common cause: copied the
test-mode secret into live-mode config, or vice versa. The mode is
encoded in the prefix — `pk_test_` and `whsec_…` from test mode go
together; same for live. Mismatch and the signature check fails.

**`Stripe not configured`** errors in Coterie — `.env` is missing
one of `STRIPE__ENABLED=true`, `STRIPE__PUBLISHABLE_KEY`,
`STRIPE__SECRET_KEY`, `STRIPE__WEBHOOK_SECRET`. All four are
required when Stripe is enabled.

**Webhook events show as "Pending" in Stripe dashboard** — Stripe
can't reach your endpoint. Either DNS doesn't resolve yet, the
firewall is blocking 443, Caddy isn't serving the host, or the
service is down. Curl the endpoint from outside the droplet:

```bash
curl -v https://coterie.your-domain.com/health
```

Should return `200 OK` and the JSON health response.

**`event_object_failed_to_deserialize` in Coterie's logs** — the
Stripe API version on the webhook doesn't match what async-stripe
expects. Re-check that you picked `2020-08-27` (or whatever the
current README says is supported); newer API versions don't
deserialize cleanly with the current `async-stripe`.

**Webhook arrives but no DB change happens** — Coterie received
the event but couldn't correlate it to a local row (e.g., a
`payment_intent.succeeded` for a `pi_*` that Coterie doesn't know
about). Usually fine — Stripe test events are synthetic and won't
match any real Coterie data. If it happens on a REAL transaction,
check that the Payment row was created BEFORE Stripe billed (the
checkout/charge flow should create a Pending row first; the
webhook flips it to Completed).

---

## Where each value goes in .env

Reference summary:

```
COTERIE__STRIPE__ENABLED=true
COTERIE__STRIPE__PUBLISHABLE_KEY=pk_test_...
COTERIE__STRIPE__SECRET_KEY=sk_test_...
COTERIE__STRIPE__WEBHOOK_SECRET=whsec_...
```

All four required when Stripe is on. When `ENABLED=false`, Coterie
skips Stripe initialization entirely and the other three values
are ignored — useful for dev / pre-DNS testing.

---

## 8. Test-mode-first deploy + one-shot switchover

This workflow is the safest way to bring a new Coterie box online if
you've never run the Stripe wiring before, or if you're verifying a
new release on a staging box, or recovering from backup.

### 8.1 Run the wizard in test mode

When invoking `sudo coterie-provision install` interactively, answer
the **"Stripe mode: [test/live]?"** prompt with `test`. The wizard
will then:

- Collect test credentials (`pk_test_*`, `sk_test_*`, test
  `whsec_*`).
- Configure `.env` with `COTERIE__DATABASE__URL` pointing at
  `coterie-test.db` instead of `coterie.db` — so test charges and
  test members live in a separate database that will be archived
  later.
- Optionally ask **"Do you also have live credentials to pre-load
  for later switchover?"** Answer `yes` if you have them ready;
  the wizard stashes them in `/opt/coterie/.env.live` (chmod 0640,
  owned by `coterie`) for the switchover to consume without
  re-prompting. Answer `no` to defer collecting them until
  switchover time.
- Print a verification checklist after the install completes.

Programmatic equivalents (for CI/IAC use):

```bash
sudo COTERIE_PROVISION_STRIPE_MODE=test \
     COTERIE_PROVISION_STRIPE_PK=pk_test_... \
     COTERIE_PROVISION_STRIPE_SK=sk_test_... \
     COTERIE_PROVISION_STRIPE_WHSEC=whsec_... \
     coterie-provision install --no-prompt …
```

Or via flags: `--stripe-mode test --stripe-publishable-key …`.

To pre-load live creds at install time, also set
`COTERIE_PROVISION_STRIPE_LIVE_PK`, `_SK`, `_WHSEC` (or the
`--stripe-live-pk` / `-sk` / `-whsec` flags).

### 8.2 Verify the test wiring

After the wizard completes, run through the checklist it prints.
At minimum:

- Confirm the box answers `/health` over HTTPS.
- Sign up a test member through the public form (or via the admin
  portal).
- Make a test donation through `/portal/donate` using card
  `4242 4242 4242 4242`, any future expiry, any 3-digit CVC, any
  ZIP.
- Confirm the charge appears in the Stripe dashboard's **TEST
  MODE** payments view.
- Confirm `journalctl -u coterie` shows
  `Webhook event received: ...` lines (Coterie successfully
  verified Stripe's webhook signature with your test `whsec_`).
- Confirm the receipt email arrived.

Repeat for any flows you care about (subscription, donation,
manual payment recording, etc.) until you're satisfied the wiring
is correct.

### 8.3 Switch to live mode

When ready:

```bash
sudo coterie-provision switch-stripe-to-live
```

This subcommand:

1. Refuses if `.env` already contains `pk_live_*` (idempotent —
   safe to run twice).
2. Refuses if `/var/lib/coterie/coterie-test.db` doesn't exist
   (not in test mode).
3. Loads live credentials from `/opt/coterie/.env.live` if you
   pre-loaded them; otherwise prompts.
4. Validates each credential prefix.
5. Calls Stripe's `/v1/balance` endpoint with the live secret key
   to confirm Stripe accepts it — **aborts before any destructive
   operation if Stripe rejects.** This catches "operator pasted
   the wrong key" before it becomes a service-down incident.
6. Asks for y/N confirmation (skip with `--yes`).
7. Stops the `coterie` service.
8. Creates a fresh `/var/lib/coterie/coterie.db` with the same
   schema as the test DB.
9. Copies the admin row(s) from `coterie-test.db` to `coterie.db`
   via `ATTACH DATABASE` — your admin password is preserved, no
   re-enter needed.
10. Archives `coterie-test.db` to
    `coterie-test-archive-YYYYMMDD-HHMMSS.db` (pass
    `--discard-test-db` to delete instead).
11. Atomically rewrites `.env` (write to `.env.new`, rename) —
    swaps the three Stripe lines and the DATABASE_URL.
12. Removes `/opt/coterie/.env.live` if it existed.
13. Starts the `coterie` service, polls `is-active` for up to 30s.
14. Smoke-tests `GET http://127.0.0.1:8080/health`.
15. Prints a success summary including the **live-mode webhook
    reminder** — see below.

CLI flags:

- `--discard-test-db` — delete `coterie-test.db` instead of
  archiving (default: archive).
- `--yes` — skip the confirmation prompt.
- `--no-prompt` — require credentials via env/flags; refuse to
  prompt interactively (for unattended runs).
- `--live-pk`, `--live-sk`, `--live-whsec` — pass live credentials
  on the command line (matching env vars:
  `COTERIE_PROVISION_STRIPE_LIVE_PK`, `_SK`, `_WHSEC`).

### 8.4 After switchover: register the LIVE webhook

The switchover prints a clear reminder, but it's worth reiterating
since this is the most common cause of "switched to live and now
payments don't work":

> **Live and test modes have completely SEPARATE webhook
> configurations in the Stripe dashboard.** The webhook you
> registered in test mode does NOT carry over.
>
> After switching to live mode, go to:
> **Stripe dashboard → toggle to LIVE mode → Developers → Webhooks**
> and re-do the registration from sections 2 and 3 of this doc,
> using the LIVE-mode `whsec_` signing secret (different from the
> test-mode one).

If you skip this, live charges will succeed at Stripe but Coterie
will never hear about them — dues won't extend, payments stay
Pending, members will think their renewal failed.
