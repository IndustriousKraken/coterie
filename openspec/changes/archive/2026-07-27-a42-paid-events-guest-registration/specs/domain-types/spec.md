# domain-types Specification

## MODIFIED Requirements

### Requirement: Payer is a sum type with Member and PublicDonor variants

`domain::Payer` SHALL be a Rust enum with variants:

- `Member(Uuid)` — existing Coterie member paying through any flow.
- `PublicDonor { name: String, email: String }` — **any non-member payer**, whose identity is captured for receipts. This covers a donor coming through `/public/donate` whose email did not match an existing member, and a guest registering and paying for an event through the public registration endpoint.

The previous flat `(member_id: Option<Uuid>, anonymous_name: Option<String>, anonymous_email: Option<String>)` shape SHALL NOT be reintroduced.

The variant name `PublicDonor` is retained for a payer who is not a donor because the variant's structure — a non-member payer with a captured name and email — already describes exactly what a guest registrant is. Renaming it would touch every donation code path for a naming improvement with no behavior in it; if the name is ever changed it SHALL be its own mechanical change, not a rider on a change that also opens a public money endpoint.

#### Scenario: Anonymous donation has no member id

- **WHEN** a public donation lacking a matching member reaches the service
- **THEN** the resulting `Payment.payer` SHALL be `Payer::PublicDonor { name, email }`, NOT `Payer::Member(None)` (which is unrepresentable)

#### Scenario: member_id() helper returns None for PublicDonor

- **WHEN** code calls `payer.member_id()`
- **THEN** for `Member(id)` it SHALL return `Some(id)`; for `PublicDonor { .. }` it SHALL return `None`

#### Scenario: A guest event registrant is a non-member payer

- **WHEN** a guest pays for an event through the public registration endpoint
- **THEN** the resulting `Payment.payer` SHALL be `Payer::PublicDonor { name, email }` carrying the guest's supplied identity, and `payer.member_id()` SHALL return `None` even when the supplied email matches an existing member
