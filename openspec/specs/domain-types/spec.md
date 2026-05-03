# domain-types Specification

## Purpose
TBD - created by archiving change document-existing-architecture. Update Purpose after archive.
## Requirements
### Requirement: Payer is a sum type with Member and PublicDonor variants

`domain::Payer` SHALL be a Rust enum with variants:

- `Member(Uuid)` — existing Coterie member paying through any flow.
- `PublicDonor { name: String, email: String }` — anonymous donor coming through `/public/donate` whose email did not match an existing member; identity captured for receipts.

The previous flat `(member_id: Option<Uuid>, anonymous_name: Option<String>, anonymous_email: Option<String>)` shape SHALL NOT be reintroduced.

#### Scenario: Anonymous donation has no member id

- **WHEN** a public donation lacking a matching member reaches the service
- **THEN** the resulting `Payment.payer` SHALL be `Payer::PublicDonor { name, email }`, NOT `Payer::Member(None)` (which is unrepresentable)

#### Scenario: member_id() helper returns None for PublicDonor

- **WHEN** code calls `payer.member_id()`
- **THEN** for `Member(id)` it SHALL return `Some(id)`; for `PublicDonor { .. }` it SHALL return `None`

### Requirement: PaymentKind is a sum type with kind-specific data on the variant

`domain::PaymentKind` SHALL be a Rust enum with variants:

- `Membership` — member dues; triggers dues-extension flow on completion.
- `Donation { campaign_id: Option<Uuid> }` — charitable donation with optional campaign id.
- `Other` — free-form (merch, event fees); no automatic side-effects.

The campaign id SHALL live on the `Donation` variant only; it SHALL NOT be a flat parallel field.

#### Scenario: Adding a payment kind requires a new variant

- **WHEN** a new payment kind is needed
- **THEN** a variant SHALL be added to `PaymentKind` and the compiler SHALL force every match site to handle it

#### Scenario: Stable as_str() mapping for DB column

- **WHEN** a `PaymentKind` is serialized to the `payment_type` DB column
- **THEN** the values SHALL be `"membership"`, `"donation"`, or `"other"` so older rows continue to deserialize

### Requirement: StripeRef is a sum type over Stripe id prefixes

`domain::StripeRef` SHALL be a Rust enum with one variant per known Stripe id prefix:

- `PaymentIntent(String)` — `pi_…` (saved-card charges, billing-runner subscriptions)
- `CheckoutSession(String)` — `cs_…` (one-time membership/donation checkouts)
- `Invoice(String)` — `in_…` (Stripe-managed subscription invoices)

The "no Stripe involvement" case SHALL be modeled as `Option<StripeRef>` on the `Payment` row (specifically `Payment.external_id: Option<StripeRef>`), NOT as a `NoStripe` variant.

#### Scenario: Manual / waived payments have external_id = None

- **WHEN** a manual or waived payment is recorded
- **THEN** the `Payment.external_id` field SHALL be `None`; no `StripeRef::NoStripe` variant SHALL exist

#### Scenario: Unknown prefix returns None at boundary

- **WHEN** `StripeRef::from_id(s)` is called with an id that doesn't start with a known prefix
- **THEN** it SHALL return `None`; the caller SHALL treat that as "not one of ours" rather than constructing a default

### Requirement: Payment uses sum types end-to-end

`domain::Payment` SHALL hold `payer: Payer`, `kind: PaymentKind`, and `external_id: Option<StripeRef>`. Repository row→domain mapping SHALL validate and construct the correct sum-type variant; internal callers SHALL trust the variant.

#### Scenario: Repo mapping rejects illegal column combinations

- **WHEN** a row is read whose nullable columns produce no valid sum-type variant (e.g., neither member id nor public-donor fields populated)
- **THEN** the repository mapper SHALL return an error; an "all-null" domain value SHALL NOT be constructed

### Requirement: Validation lives at boundaries; internal code trusts types

Validation (length limits, enum membership, structural invariants) SHALL happen at boundaries: HTTP handlers, repository row mapping, webhook payload mapping. Internal services and downstream code SHALL trust the validated types.

#### Scenario: Service does not re-validate sum-type variants

- **WHEN** `PaymentService::record_manual` receives a `RecordManualPaymentInput` with a typed `PaymentKind`
- **THEN** the service SHALL NOT re-check whether the kind is one of a hardcoded set; the type system already guarantees that

#### Scenario: Service does validate domain-level constraints

- **WHEN** `PaymentService::record_manual` receives an `amount_cents` value
- **THEN** the service SHALL check `amount_cents >= 0` and `amount_cents <= MAX_PAYMENT_CENTS` because those are domain rules not encoded in the type system

### Requirement: New domain types live in src/domain/

Domain types SHALL be defined in `src/domain/` and used end-to-end (handler → service → repository → row mapping). Inventing parallel "DTOs" with the same shape SHALL be avoided unless a wire-format constraint requires it.

#### Scenario: Adding a value object updates all layers

- **WHEN** a new value object (e.g., a money amount with currency) is introduced
- **THEN** the type SHALL be defined in `src/domain/` and used at every layer that handles that value

