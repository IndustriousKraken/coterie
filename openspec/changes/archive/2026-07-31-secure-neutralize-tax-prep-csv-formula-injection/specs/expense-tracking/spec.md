# expense-tracking Specification

## ADDED Requirements

### Requirement: Tax-prep CSV export neutralizes spreadsheet formula injection

The tax-prep CSV export (`GET /portal/admin/finance/reports/tax-prep`) SHALL neutralize spreadsheet formula injection (CWE-1236) in its externally-influenced free-text columns (`description`, `counterparty`, `category`, `account`). `counterparty` and `description` carry values an unauthenticated caller supplies — the donor name and email a public donation is recorded with, and a member's own chosen full name — so they are attacker-controlled, not operator-controlled.

Specifically, when one of these field values begins with a formula-trigger character (`=`, `+`, `-`, `@`) or with a control character a spreadsheet may treat as starting a formula (TAB, CR, LF), the export SHALL prefix the value with a single quote (`'`) inside the RFC 4180 double-quoting so spreadsheet applications render the cell as literal text rather than evaluating it. The server-controlled columns (`date`, `type`, `amount`, `reference`) SHALL NOT be altered by this neutralization — in particular a refund's negative `amount` SHALL still export as a leading-minus number.

#### Scenario: A donor-supplied name that starts with a formula trigger is neutralized

- **GIVEN** a completed public donation whose recorded donor name is `=HYPERLINK("http://evil","x")`
- **WHEN** an admin exports the tax-prep CSV for that year
- **THEN** the `counterparty` and `description` fields SHALL each begin with a single quote immediately after the opening double-quote (e.g. `"'=HYPERLINK(...)"`) so a spreadsheet opens them as text, not a formula

#### Scenario: An expense category or account name that starts with a formula trigger is neutralized

- **GIVEN** an expense in the exported year whose category name begins with `+`
- **WHEN** the tax-prep CSV is generated
- **THEN** that row's `category` field SHALL begin with a single quote immediately after the opening double-quote

#### Scenario: Ordinary values and server-controlled columns are unchanged

- **WHEN** a row's `counterparty` is `O'Brien, Sean` (no leading formula trigger) and its `type` is `refund` with an `amount` of `-25.00`
- **THEN** the `counterparty` field SHALL export as `"O'Brien, Sean"` with no injected single quote, and the `amount` field SHALL export as `"-25.00"` with no injected single quote
