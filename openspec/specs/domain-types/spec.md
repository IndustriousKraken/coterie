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

### Requirement: BasicType collapses event-type and announcement-type into one struct

`domain::BasicType` SHALL be a single Rust struct holding the fields shared by event types and announcement types: `id`, `name`, `slug`, `description`, `color`, `icon`, `sort_order`, `is_active`, `created_at`, `updated_at`. The kind discriminator (`BasicTypeKind`) SHALL NOT be stored on the struct itself — it lives on the service / repository / handler that produced or consumes the value, because the two type lists are physically separate tables.

`EventTypeConfig` and `AnnouncementTypeConfig` SHALL become type aliases for `BasicType` so existing call sites continue to compile and read naturally at the API boundary.

#### Scenario: BasicType has no kind field on the row

- **WHEN** code reads a `BasicType` value
- **THEN** the value SHALL NOT carry a kind discriminator on the struct itself; the kind is implicit in which service/repository the value came from

#### Scenario: Old type names continue to be importable

- **WHEN** existing code imports `EventTypeConfig` or `AnnouncementTypeConfig`
- **THEN** the import SHALL continue to resolve via type aliases and SHALL refer to the same `BasicType` underneath

### Requirement: Request shapes unify with type aliases

`CreateBasicTypeRequest` and `UpdateBasicTypeRequest` SHALL replace the four parallel request structs (`CreateEventTypeRequest`, `CreateAnnouncementTypeRequest`, `UpdateEventTypeRequest`, `UpdateAnnouncementTypeRequest`). The old names SHALL remain as type aliases.

`MembershipType`'s request shapes SHALL stay separate — they carry `fee_cents` and `billing_period` fields not present on the basic shape.

#### Scenario: Old request type names continue to be importable

- **WHEN** existing code references `CreateEventTypeRequest` or `UpdateAnnouncementTypeRequest`
- **THEN** the reference SHALL resolve via type alias to the unified `CreateBasicTypeRequest` / `UpdateBasicTypeRequest`

### Requirement: BasicTypeKind is a closed enum with const accessors

`domain::BasicTypeKind` SHALL be a closed Rust enum with variants `Event` and `Announcement`. The enum SHALL expose const-equivalent accessors (`table()`, `usage_table()`, `usage_fk()`, `display_name()`) returning `&'static str` so SQL strings and error messages can be built without runtime branching at every call site.

The kind SHALL NOT be extended to admit user-controlled values. Adding a new variant SHALL force every accessor to return a value for it (the compiler enforces totality on the `match` expressions inside the accessors).

#### Scenario: SQL strings interpolate kind.table() safely

- **WHEN** the basic-type repository builds a SQL statement
- **THEN** it SHALL interpolate the `&'static str` from `kind.table()` (and similar accessors); the value SHALL NOT come from user input or runtime configuration

#### Scenario: Adding a new kind forces every accessor to be updated

- **WHEN** a contributor adds a new `BasicTypeKind` variant
- **THEN** the compiler SHALL fail to build until every const accessor (`table`, `usage_table`, `usage_fk`, `display_name`) returns a value for the new variant

### Requirement: MemberStatus exposes typed predicate helpers

`domain::MemberStatus` SHALL expose `pub fn is_active(self) -> bool`, `is_pending`, `is_expired`, `is_suspended`, and `is_honorary` predicate methods. These methods exist so callers — especially Askama templates rendering `MemberStatus` values from `MemberInfo` / `AdminMemberInfo` projections — can branch on status without comparing against string literals. A typo in a predicate name SHALL be caught at compile time; a renamed `MemberStatus` variant SHALL force every predicate to be updated.

`MemberStatus` SHALL also derive `Copy` so the predicates can be called on owned values without `&` ceremony at the template call site.

#### Scenario: Templates branch on typed predicates

- **WHEN** a template renders a member context and needs to branch on the member's status
- **THEN** it SHALL call `member.status.is_active()` (or the relevant predicate) rather than comparing against a string literal like `member.status == "Active"`

#### Scenario: Renaming a variant catches drift at compile time

- **WHEN** a contributor renames `MemberStatus::Pending` (e.g., to `Submitted`)
- **THEN** the corresponding `is_pending` predicate SHALL be updated as part of the rename or the build SHALL fail; templates calling `is_pending` SHALL fail to render against the new variant set, surfacing the drift immediately rather than silently routing the wrong branch

#### Scenario: Predicates are total over the variant set

- **WHEN** a contributor adds a new variant to `MemberStatus`
- **THEN** they SHALL also add the corresponding `is_<variant>` predicate so every variant is observable from templates without falling back to string comparison

### Requirement: Member-context template projections carry typed status and dates

The presentation projections `MemberInfo` (in `src/web/portal/mod.rs`) and `AdminMemberInfo` (in `src/web/portal/admin/members.rs`) SHALL hold:

- `id: Uuid` (not `String`)
- `status: MemberStatus` (not `String`)
- `joined_at: DateTime<Utc>` (not pre-formatted `String`)
- `dues_paid_until: Option<DateTime<Utc>>` (not `Option<String>`)

Date formatting SHALL be applied at the template layer via Askama filters (`fmt_long_date`, `fmt_short_date`, plus `_opt` variants for `Option<DateTime<Utc>>`). Per-handler `format!("%B %d, %Y")` calls in projection construction sites SHALL be removed.

The projections SHALL remain distinct types from `domain::Member` so that `Member` fields not safe to render to member-facing pages (`notes`, `stripe_customer_id`, `stripe_subscription_id`, `discord_id`) cannot leak into a template via the projection.

#### Scenario: Construction sites pass typed values directly

- **WHEN** a handler constructs `MemberInfo` from a `Member`
- **THEN** the construction SHALL move `member.status`, `member.joined_at`, `member.dues_paid_until`, and `member.id` straight onto the projection without per-field `format!` or `.as_str()` conversion

#### Scenario: Templates format dates via the filter, not via pre-formatted strings

- **WHEN** a template renders `member.joined_at`
- **THEN** it SHALL apply the appropriate filter (`{{ member.joined_at|fmt_long_date }}` or `|fmt_short_date`); the projection SHALL NOT carry a pre-formatted string

#### Scenario: Adding a new render-shielded field to Member does not leak

- **WHEN** a contributor adds a new field to `domain::Member` (e.g., a private admin note)
- **THEN** the field SHALL NOT automatically appear on `MemberInfo` / `AdminMemberInfo`; the contributor must explicitly add it to a projection if it should render

