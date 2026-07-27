# domain-types Specification

## MODIFIED Requirements

### Requirement: PaymentKind is a sum type with kind-specific data on the variant

`domain::PaymentKind` SHALL be a Rust enum with variants:

- `Membership` — member dues; triggers dues-extension flow on completion.
- `Donation { campaign_id: Option<Uuid> }` — charitable donation with optional campaign id.
- `EventFee { event_id: Uuid }` — a fee paid to attend one event; confirms the payer's seat on that event and triggers no dues-extension flow.
- `SeriesPass { series_id: Uuid }` — a single payment buying enrollment in every remaining occurrence of a bounded recurring series; confirms the payer's enrollment and triggers no dues-extension flow.
- `Other` — free-form (merch, miscellaneous); no automatic side-effects.

The campaign id SHALL live on the `Donation` variant only; it SHALL NOT be a flat parallel field. The event id SHALL likewise live on the `EventFee` variant only, and the series id on the `SeriesPass` variant only.

`EventFee` SHALL carry its `event_id` rather than being recorded as `Other`, for three reasons: a `charge.refunded` webhook arrives with a payment id and nothing else, so releasing the correct seat requires the event id to be reachable from the payment; `Other` is specified to have no automatic side-effects, which an event fee does have; and event revenue must be separable from dues and donations in reporting rather than landing in an untyped bucket.

`SeriesPass` SHALL be a sibling variant rather than a generalization of `EventFee` into a target enum. One seat at one occurrence and a pass to a whole course are different products with different refund and capacity rules; two variants each carrying the id they need keep every match site explicit and let the compiler enforce totality when either is handled.

#### Scenario: Adding a payment kind requires a new variant

- **WHEN** a new payment kind is needed
- **THEN** a variant SHALL be added to `PaymentKind` and the compiler SHALL force every match site to handle it

#### Scenario: Stable as_str() mapping for DB column

- **WHEN** a `PaymentKind` is serialized to the `payment_type` DB column
- **THEN** the values SHALL be `"membership"`, `"donation"`, `"event_fee"`, `"series_pass"`, or `"other"` so older rows continue to deserialize

#### Scenario: Existing rows are unaffected by the added variant

- **WHEN** a payment row written before a variant existed is read back
- **THEN** its stored `payment_type` SHALL continue to deserialize to the same variant as before; the mapping SHALL be additive only

#### Scenario: An event fee is reachable from the payment to its event

- **WHEN** code holds a `PaymentKind::EventFee { event_id }`
- **THEN** the event id SHALL be readable from the kind without a side lookup table, so a refund can locate and release the corresponding seat

#### Scenario: A series pass is reachable from the payment to its series

- **WHEN** code holds a `PaymentKind::SeriesPass { series_id }`
- **THEN** the series id SHALL be readable from the kind without a side lookup table, so a refund can locate the enrollment and cancel its remaining seats
