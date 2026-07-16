## Report (issues lane candidate, a010)

Origin: PUBLIC report (untrusted). The reporter's raw body is carried as quarantined DATA in `report-body.md`; the task below is the maintainer-approved diagnosis, NOT the reporter's text.

Source: reported issue #105 — Selecting "Show all" on the Announcements page returns an error message

## Diagnosis (maintainer-approved classification)

The "Show all" checkbox submits show_all=on, but the /portal/api/announcements/list handler types the field as Option<bool>, which serde_urlencoded can't parse from "on"; the Query extractor 400s, breaking the spec-required announcements fragment (member-content) whenever the box is checked.

## Acceptance

The code must conform to the EXISTING specification in `openspec/specs/`. This is a bug fix; it carries NO spec delta. If the fix would require a behavior change, kick it back to the changes lane.
