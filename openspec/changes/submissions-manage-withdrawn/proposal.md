# submissions-manage-withdrawn

## Why

Reported in GitHub issue #121. The submissions feature makes `withdrawn` a dead
end: a member who withdraws a proposal can neither remove it from their "My
Submissions" list nor revise and resubmit it (editing is allowed only while
`submitted`). A withdrawal by accident, or a "let me fix this and try again,"
has no recourse.

Give the owner two actions on their own terminal submissions: **delete** a
`withdrawn` or `declined` one (remove it from the list, clean up its attachment),
and **re-open** a `withdrawn` one back to `submitted` for revision/resubmission.
This changes the submission lifecycle, so it needs a proposal.

## What Changes

- **Delete (owner):** a member MAY delete their own submission when it is in a
  terminal `withdrawn` or `declined` state — the row is removed and its
  attachment (if any) deleted. Delete of a `submitted`/`under_review`/`accepted`/
  `scheduled` submission is refused.
- **Re-open (owner):** a member MAY re-open their own `withdrawn` submission back
  to `submitted`, after which it is editable again (and re-reviewable). Re-open is
  allowed only from `withdrawn` — a `declined` submission is not resurrected
  (make a fresh submission instead), preserving the reviewer's decision.
- **Access control unchanged:** both actions are strictly owner-scoped and
  deny-without-disclosure for non-owners; admins remain exempt for review. CSRF
  applies as to any state-changing submission request.

## Impact

- **Spec:** `submissions` — 1 MODIFIED requirement ("A member can access only
  their own submissions"): its three existing scenarios still hold (editing a
  non-`submitted` submission directly is still refused — re-open is a separate
  action); adds delete + re-open behavior and scenarios.
- **Code:** member submission routes/handlers (`src/web/portal/submissions.rs`)
  gain owner-scoped `delete` and `reopen` actions; the service
  (`src/service/submission_service/`) enforces the state guard and deletes the
  attachment on delete (reusing `delete_uploaded_file`); the "My Submissions"
  template shows Delete/Re-open buttons on the appropriate states.
- **Tests:** owner deletes withdrawn/declined (row + attachment gone); owner
  re-opens withdrawn → `submitted` and editable; delete/re-open of a non-terminal
  or non-`withdrawn` state refused; a non-owner's delete/re-open denied.
