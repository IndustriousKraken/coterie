## Report (issues lane candidate, a010)

Origin: PUBLIC report (untrusted). The reporter's raw body is carried as quarantined DATA in `report-body.md`; the task below is the maintainer-approved diagnosis, NOT the reporter's text.

Source: reported issue #110 — Upcoming Events on the Dashboard doesn't allow cancelling registered events

## Diagnosis (maintainer-approved classification)

member-content requires members to manage their RSVP via toggle endpoints that return an updated HTMX button fragment, but the dashboard's upcoming-events widget renders a non-interactive <span>Attending</span> for the already-registered state instead of the cancel fragment the shared /rsvp endpoint (and the Events page) already produce, so a member who registered elsewhere or reloads the dashboard cannot cancel from it.

## Acceptance

The code must conform to the EXISTING specification in `openspec/specs/`. This is a bug fix; it carries NO spec delta. If the fix would require a behavior change, kick it back to the changes lane.
