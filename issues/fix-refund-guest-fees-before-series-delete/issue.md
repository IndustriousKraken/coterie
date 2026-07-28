# Bulk series delete strands guests' paid event fees

## What is wrong

`admin_delete_event` refuses a series-wide `end_series` / `delete_series`
when the occurrences carry a per-night price, because those sibling
occurrences are dropped by the cascade and this handler never sees them —
so it cannot guarantee their attendees were refunded first. The guard
reads only the **member** price:

`src/web/portal/admin/events/single.rs:1093-1109`

```rust
if (scope == "end_series" || scope == "delete_series") && series_id.is_some() {
    if event.is_paid_for_members() {
        return partials::admin_alert(
            "error",
            "This series' occurrences have a member price. Delete them one at a time so \
             each event's attendees are refunded first.",
            false,
        ).into_response();
    }
    let sid = series_id.unwrap();
    // ... refund_all_series_passes(sid) ... then end_series / delete_series
```

`Event::is_paid_for_members()` is `member_price_cents > 0`
(`src/domain/event.rs:63`). A guest fee lives in a different column —
`Event::is_paid_for_guests()` is `guest_price_cents > 0`
(`src/domain/event.rs:70`) — and the two are deliberately independent
("Free for members, paid for the public" is a supported combination,
`openspec/specs/paid-events/spec.md:381`).

So a series whose occurrences are **free for members and priced for
guests** passes the guard. The only refund sweep on this path is
`refund_all_series_passes` (`single.rs:1120`), which refunds
`PaymentKind::SeriesPass` rows only. The guests' `PaymentKind::EventFee`
payments — one `Completed` row per guest per occurrence, taken through
`POST /public/events/:id/register` — are never touched:
`refund_all_event_fees` (`single.rs:1184`) is per-event and is only
reached on the `scope == "this"` path below.

`delete_series` then cascade-deletes the series row and, via the FK
cascade, every occurrence and its `event_attendance` rows
(`src/service/event_admin_service.rs:378-392`). `end_series` deletes every
future occurrence the same way (`:344-373`).

## Who triggers it, and how

An admin, through the normal UI — no malice required:

1. Create a recurring public workshop: `member_price_cents = 0`,
   `guest_price_cents = 2500`, `guest_registration_enabled = true`,
   visibility `Public`.
2. Guests register and pay $25 each at `/events/:id/register`; each gets a
   `Completed` `EventFee` payment and a `Registered` seat.
3. The series is cancelled. The admin opens any occurrence and chooses
   "delete series" (or "end series").
4. The guard passes (`member_price_cents == 0`), the series-pass sweep
   finds nothing, and every occurrence plus its roster is deleted.

## Harm

Real money kept with nothing delivered, and the record of who was owed it
destroyed in the same statement: `event_attendance` cascades away, so
after the delete there is no roster to reconcile against. This is
precisely the failure the refund-before-delete ordering exists to prevent
— "a delete-then-refund ordering would destroy the roster while the
charges stood, leaving unrefundable money and no record of who was owed
it".

## Source locations

- `src/web/portal/admin/events/single.rs:1101` — the one-sided guard.
- `src/web/portal/admin/events/single.rs:1120` — the sweep that runs
  instead (series passes only).
- `src/service/event_admin_service.rs:344` / `:378` — `end_series` /
  `delete_series`, both of which drop occurrences.

## Acceptance criteria (against the EXISTING specification)

No spec delta. The fix makes the code conform to requirements that
already exist in `openspec/specs/paid-events/spec.md`:

- **Requirement: Deleting a paid event refunds every paid attendee before
  the event is removed** — "Deleting an event that has `Completed`
  event-fee payments SHALL refund every such payment first and SHALL abort
  the deletion if any refund fails … An event that cannot be fully
  refunded SHALL remain visible and fixable rather than becoming an
  invisible pile of unreturned charges." A guest's fee is a `Completed`
  event-fee payment; deleting its occurrence through the series scope
  neither refunds it nor aborts.
- **Requirement: Guests are identified as guests on the roster and in
  reporting**, scenario *"A guest seat can be refunded and released like a
  member seat"* — a guest fee is refundable on the same terms as a
  member's, so it is covered by the requirement above.

Concretely, after the fix:

1. A series-scope `end_series` / `delete_series` SHALL be refused, with
   the existing "delete them one at a time" alert, whenever any occurrence
   of the series has a `Completed` event-fee payment — whether the payer
   is a member or a guest. No occurrence, attendance row, or series row is
   removed by a refused call.
2. The refusal SHALL be driven by the presence of `Completed` event-fee
   payments, not by a price column: a series priced but never bought is
   still bulk-deletable, and a series whose price was later set to `0`
   with paid seats outstanding is not.
3. The existing `refund_all_series_passes`-then-delete behavior for class
   passes, and the per-occurrence `refund_all_event_fees` sweep on
   `scope == "this"`, are unchanged.
