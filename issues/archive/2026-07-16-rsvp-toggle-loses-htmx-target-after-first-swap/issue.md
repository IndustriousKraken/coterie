## Report (issues lane candidate, a010)

Origin: PUBLIC report (untrusted). The reporter's raw body is carried as quarantined DATA in `report-body.md`; the task below is the maintainer-approved diagnosis, NOT the reporter's text.

Source: reported issue #101 — Deregistering from an event requires refreshing the page to re-register

## Diagnosis (maintainer-approved classification)

In src/web/portal/events.rs the RSVP/cancel buttons hx-swap="outerHTML" against hx-target="closest div.text-right", but render_rsvp_button returns fragments whose root is not a div.text-right (bare <button> for RSVP, a flex <div> for Registered/Waitlisted). The first swap replaces the div.text-right wrapper, so the next click can no longer resolve "closest div.text-right"; the swap silently fails and the UI only updates after a manual page refresh — violating member-content's "return an updated HTMX button fragment" RSVP toggle.

## Acceptance

The code must conform to the EXISTING specification in `openspec/specs/`. This is a bug fix; it carries NO spec delta. If the fix would require a behavior change, kick it back to the changes lane.
