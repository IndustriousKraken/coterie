## Report (issues lane candidate, a010)

Origin: PUBLIC report (untrusted). The reporter's raw body is carried as quarantined DATA in `report-body.md`; the task below is the maintainer-approved diagnosis, NOT the reporter's text.

Source: reported issue #109 — Selecting "Show Past events" on the Events page returns an error message

## Diagnosis (maintainer-approved classification)

The "Show past events" checkbox submits show_past=on, but the /portal/api/events/list handler types EventsListQuery.show_past as Option<bool>, which serde_urlencoded can't parse from "on"; the Query extractor 400s, breaking the spec-required events fragment (member-content) whenever the box is checked. Actually displaying past events is a separate behavior change and out of scope.

## Acceptance

The code must conform to the EXISTING specification in `openspec/specs/`. This is a bug fix; it carries NO spec delta. If the fix would require a behavior change, kick it back to the changes lane.
