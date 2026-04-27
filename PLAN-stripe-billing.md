# Stripe Billing Overhaul - Implementation Plan

> **Status: Implemented.** This plan is preserved for design rationale.
> The active code lives in `src/payments/stripe_client.rs`,
> `src/service/billing_service.rs`, `src/jobs/billing_runner.rs`,
> `src/repository/{payment,saved_card,scheduled_payment,donation}_repository.rs`,
> `src/web/portal/{payments,donations}.rs`, and the migrations in
> `migrations/` (notably 002 onward).
>
> **Items NOT built** (intentionally, or deferred — see TODO.md):
> - **Public donation page** (Phase 4). Decided against hosting in
>   Coterie; public donation forms live on the frontend site and POST
>   to a forthcoming `POST /public/donate` API endpoint instead.
> - **Recurring donations** (Phase 4, marked "optional" in the plan).
> - **Standalone admin billing dashboard** (Phase 5). The current
>   `admin/billing.rs` is a settings page; the dashboard with upcoming
>   payments / recent failures / revenue metrics wasn't built.
> - **Stripe Tax** (Open Question 5). Out of scope.

## Overview

Replace redirect-based Stripe Checkout with embedded Stripe Elements. Add Coterie-managed recurring billing, support for legacy Stripe subscriptions during migration, and donation support.

**Core principle**: Coterie is the source of truth for billing. Stripe is a payment processor. This enables future support for other processors.

---

## Phase 1: Stripe Elements & Payment Methods

**Goal**: Members can enter card details on-site, save payment methods, make one-time payments.

### Database Changes

```sql
-- Add to members table
ALTER TABLE members ADD COLUMN stripe_customer_id TEXT;
ALTER TABLE members ADD COLUMN stripe_subscription_id TEXT;  -- Legacy only
ALTER TABLE members ADD COLUMN billing_mode TEXT NOT NULL DEFAULT 'manual';
-- billing_mode: 'coterie_managed', 'stripe_subscription', 'manual'

-- New table: saved payment methods
CREATE TABLE payment_methods (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL REFERENCES members(id),
    stripe_payment_method_id TEXT NOT NULL,
    card_last_four TEXT NOT NULL,
    card_brand TEXT NOT NULL,
    exp_month INTEGER NOT NULL,
    exp_year INTEGER NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_payment_methods_member ON payment_methods(member_id);
```

### Backend Changes

1. **New endpoint**: `POST /api/payments/setup-intent`
   - Creates a Stripe SetupIntent for saving a card
   - Returns `client_secret` for Stripe.js

2. **New endpoint**: `POST /api/payments/confirm-setup`
   - Called after Stripe.js confirms the SetupIntent
   - Saves PaymentMethod to our database
   - Creates Stripe Customer if member doesn't have one

3. **New endpoint**: `GET /api/payments/methods`
   - List member's saved payment methods

4. **New endpoint**: `DELETE /api/payments/methods/:id`
   - Remove a saved payment method

5. **New endpoint**: `POST /api/payments/methods/:id/default`
   - Set a payment method as default

6. **Update checkout flow**:
   - If member has saved payment method: show "Pay with •••• 4242" button
   - If not: show Stripe Elements card form
   - Charge via PaymentIntent (not Checkout Session)

### Frontend Changes

1. **Add Stripe.js** to base layout (or payment pages)
2. **New component**: Card input form using Stripe Elements
3. **Update `/portal/payments/new`**:
   - Show saved cards if any
   - Show "Add new card" option with Elements form
   - On submit: charge immediately, extend dues

### Files to Create/Modify

- `migrations/XXXXXX_payment_methods.sql` (new)
- `src/domain/payment_method.rs` (new)
- `src/repository/payment_method.rs` (new)
- `src/payments/stripe_client.rs` (add SetupIntent, PaymentIntent methods)
- `src/api/handlers/payments.rs` (add new endpoints)
- `src/web/portal/payments.rs` (update handlers)
- `templates/portal/payment_new.html` (Stripe Elements integration)
- `templates/layouts/base.html` (add Stripe.js)

---

## Phase 2: Coterie-Managed Recurring Billing

**Goal**: Coterie controls when to charge members. Auto-renewal with saved cards.

### Database Changes

```sql
-- Scheduled payments (Coterie's billing queue)
CREATE TABLE scheduled_payments (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL REFERENCES members(id),
    membership_type_id TEXT NOT NULL,
    amount_cents INTEGER NOT NULL,
    due_date TEXT NOT NULL,  -- Date only, not datetime
    status TEXT NOT NULL DEFAULT 'pending',
    -- status: 'pending', 'processing', 'completed', 'failed', 'canceled'
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT,
    payment_id TEXT REFERENCES payments(id),
    failure_reason TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_scheduled_payments_due ON scheduled_payments(due_date, status);
CREATE INDEX idx_scheduled_payments_member ON scheduled_payments(member_id);

-- Settings for billing behavior
-- (Uses existing settings table)
-- billing.grace_period_days: 3
-- billing.max_retry_attempts: 3
-- billing.retry_interval_days: 3
-- billing.dunning_email_1_days: 0  (on first failure)
-- billing.dunning_email_2_days: 3  (before final attempt)
-- billing.dunning_email_final_days: 7  (membership suspended)
```

### Backend Changes

1. **Background job**: `BillingRunner`
   - Runs on schedule (configurable, default: every hour)
   - Finds `scheduled_payments` where `due_date <= today` and `status = pending`
   - Attempts charges, handles success/failure
   - Creates next scheduled payment on success
   - Implements retry logic on failure

2. **New service**: `BillingService`
   - `schedule_renewal(member_id, membership_type_id)` — creates scheduled_payment
   - `cancel_scheduled_payments(member_id)` — cancels pending payments
   - `process_scheduled_payment(id)` — attempts the charge

3. **Member status transitions**:
   - When `dues_paid_until` passes + grace period → status becomes Expired
   - Background job checks this daily

4. **Email notifications** (templates configurable):
   - Payment successful
   - Payment failed (with retry info)
   - Final warning before suspension
   - Membership suspended

### Files to Create/Modify

- `migrations/XXXXXX_scheduled_payments.sql` (new)
- `src/domain/scheduled_payment.rs` (new)
- `src/repository/scheduled_payment.rs` (new)
- `src/service/billing_service.rs` (new)
- `src/jobs/billing_runner.rs` (new)
- `src/jobs/mod.rs` (new - job framework)
- `src/main.rs` (spawn background jobs)
- Seed default settings for billing config

---

## Phase 3: Legacy Stripe Subscription Support

**Goal**: Existing Stripe subscriptions continue working. Coterie listens to webhooks and credits accounts.

### Webhook Handlers

Expand `handle_webhook` in `stripe_client.rs`:

| Event | Action |
|-------|--------|
| `invoice.paid` | Find member by `stripe_customer_id`, extend `dues_paid_until`, create payment record |
| `invoice.payment_failed` | Log warning, Stripe handles retries |
| `customer.subscription.deleted` | Clear member's `stripe_subscription_id`, set `billing_mode = manual` |
| `customer.subscription.updated` | Update any cached subscription info |
| `payment_method.automatically_updated` | Update our `payment_methods` table (card replaced by bank) |

### Transition Logic

When a legacy subscription ends (canceled, failed permanently, etc.):
1. Clear `stripe_subscription_id`
2. Set `billing_mode = manual`
3. Member's next payment goes through Coterie flow
4. If they save a card → `billing_mode = coterie_managed`

### Admin Tools

- View member's Stripe subscription status
- "Convert to Coterie billing" button:
  1. Cancel Stripe subscription via API
  2. Import their payment method if possible
  3. Set up Coterie-managed billing from current `dues_paid_until`

---

## Phase 4: Donations

**Goal**: One-time payments not tied to membership dues.

### Database Changes

```sql
-- Add payment_type to distinguish
ALTER TABLE payments ADD COLUMN payment_type TEXT NOT NULL DEFAULT 'membership';
-- payment_type: 'membership', 'donation', 'other'

-- Optional: donation campaigns/funds
CREATE TABLE donation_campaigns (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    description TEXT,
    goal_cents INTEGER,  -- Optional fundraising goal
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

### Frontend

- `/portal/donate` — donation page
  - Optional: select campaign/fund
  - Enter amount (preset buttons + custom)
  - Use saved card or enter new
  - Optional: make it recurring (monthly donation)
- Public donation page (no login required): `/donate`
  - Collects email, name, amount
  - Creates a "donor" record or links to existing member

### Backend

- `POST /api/payments/donate` — process donation
- Donation payments: `payment_type = 'donation'`, no dues extension
- Optional: recurring donations (separate from membership billing)

---

## Phase 5: Migration & Admin Tools

**Goal**: Admins can manage the transition, bulk operations.

### Admin Features

1. **Member billing details page**:
   - View billing mode, saved cards, scheduled payments
   - View Stripe subscription if legacy
   - Actions: add payment, convert to Coterie, cancel scheduled payments

2. **Bulk migration tool**:
   - List all members with `billing_mode = stripe_subscription`
   - "Convert all" or selective conversion
   - Preview what will happen

3. **Billing dashboard**:
   - Upcoming scheduled payments
   - Recent failures
   - Revenue metrics

---

## Configuration (Settings)

Add to settings system:

```
billing.grace_period_days = 3
billing.max_retry_attempts = 3
billing.retry_interval_days = 3
billing.auto_renew_default = true

email.payment_success_subject = "Payment received - thank you!"
email.payment_success_body = "..." (template with {{member_name}}, {{amount}}, etc.)
email.payment_failed_subject = "Payment failed - action required"
email.payment_failed_body = "..."
email.membership_suspended_subject = "Your membership has been suspended"
email.membership_suspended_body = "..."
```

---

## Implementation Order

1. **Phase 1a**: Database migrations, PaymentMethod domain/repo
2. **Phase 1b**: Stripe SetupIntent/PaymentIntent in stripe_client.rs
3. **Phase 1c**: API endpoints for payment methods
4. **Phase 1d**: Stripe Elements frontend integration
5. **Phase 2a**: ScheduledPayment domain/repo, BillingService
6. **Phase 2b**: BillingRunner background job
7. **Phase 2c**: Member status transitions, grace period
8. **Phase 2d**: Email notifications (basic)
9. **Phase 3**: Legacy subscription webhooks
10. **Phase 4**: Donations
11. **Phase 5**: Admin tools

---

## Testing Strategy

### Local Testing
- Stripe test mode with `sk_test_` keys
- Stripe CLI for webhook forwarding
- Test card numbers: `4242424242424242` (success), `4000000000000002` (decline)

### Subscription Testing
- Create test subscriptions in Stripe Dashboard
- Use Stripe CLI to trigger webhook events:
  ```bash
  stripe trigger invoice.paid
  stripe trigger customer.subscription.deleted
  ```

### Billing Runner Testing
- Manually set `due_date` to today
- Run billing job manually via admin endpoint or CLI
- Verify charges appear in Stripe Dashboard

---

## Open Questions

1. **Publishable key storage**: Add `COTERIE__STRIPE__PUBLISHABLE_KEY` to config for frontend Stripe.js?

2. **Email sending**: Do we have email infrastructure? Or stub it for now?

3. **Idempotency**: Stripe recommends idempotency keys for charges. Generate from `scheduled_payment.id`?

4. **Currency**: Currently hardcoded to USD. Make configurable per-org?

5. **Tax**: Stripe Tax integration, or out of scope for now?

---

## Estimated Effort

- Phase 1: 2-3 sessions (Stripe Elements is fiddly)
- Phase 2: 2-3 sessions (background jobs, state machines)
- Phase 3: 1 session (mostly webhook handlers)
- Phase 4: 1 session (simpler than subscriptions)
- Phase 5: 1-2 sessions (admin UI)

Total: ~8-12 working sessions depending on complexity discovered.
