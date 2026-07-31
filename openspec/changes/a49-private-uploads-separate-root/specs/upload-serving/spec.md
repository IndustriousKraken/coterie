# upload-serving Specification

## ADDED Requirements

### Requirement: The public upload route decides by allow-list, not deny-list

`GET /uploads/:filename` SHALL determine eligibility by asking whether the file is
known to be **public**, and SHALL refuse anything it cannot affirmatively confirm.
The predicate SHALL be phrased positively — a public-image lookup — and the
absence of a match SHALL mean deny, never serve.

The route SHALL NOT ask whether a file is private and serve everything else. That
phrasing makes disclosure the default outcome for any file the query does not
recognise, and a query stops recognising a file for reasons that have nothing to
do with intent: the row was deleted, the row was cascaded away with its owner, the
column was updated to point elsewhere, or the row has not been committed yet.
Under a deny-list each of those publishes the file; under an allow-list each of
them denies it.

The two phrasings are equally cheap and differ only in which way they fail. One
fails toward disclosure and the other toward a broken image, so the choice is not
a trade-off.

This inversion SHALL also govern how the rule extends. A future upload category
whose author forgets to register it in the allow-list produces a visibly broken
asset, which is noticed and fixed. The same omission under a deny-list produces a
silently world-readable file, which is noticed by whoever finds it first. A
mistake that announces itself is strictly preferable to one that does not.

Non-public event and announcement images SHALL therefore require an authenticated
session as they do today, but by falling through the allow-list rather than by
matching a private-list.

#### Scenario: An image whose row was deleted is no longer served publicly

- **WHEN** an event or announcement is deleted and its image file remains on disk,
  and an anonymous caller requests that filename
- **THEN** the request SHALL be refused, because nothing affirms the file is
  public

#### Scenario: A public event's image is still served anonymously

- **WHEN** an anonymous caller requests the image of an event whose visibility is
  `Public`
- **THEN** the file SHALL be served, as it is today

#### Scenario: A members-only image still requires a session

- **WHEN** an anonymous caller requests the image of a `MembersOnly` event
- **THEN** the request SHALL be refused; an authenticated caller SHALL receive it

#### Scenario: An unregistered upload category fails visibly, not silently

- **WHEN** a new kind of upload is added without being registered in the
  public-image allow-list
- **THEN** requests for it SHALL be refused rather than served, so the omission
  surfaces as a broken asset rather than as an unnoticed disclosure

### Requirement: Visibility changes need no file movement

A visibility change between public and non-public SHALL NOT require the item's
image file to be moved, renamed, or re-uploaded; the allow-list query SHALL simply
return a different answer for the same file.

This is why the allow-list is the right mechanism for images specifically:
visibility is **mutable**, so any scheme that encodes public-versus-private into
the file's location would have to relocate files on every transition and would
leave a window — and a stale URL — on each one. Attachments, whose privacy is
fixed for the life of the file, are handled by storage separation instead; images,
whose privacy is a mutable property of a row, are handled by a fail-closed query
against that row.

#### Scenario: Flipping an event to members-only takes effect without touching disk

- **WHEN** an admin changes an event from `Public` to `MembersOnly`
- **THEN** its image SHALL stop being served to anonymous callers immediately, with
  no file move and no path rewrite

#### Scenario: Flipping back to public restores anonymous access

- **WHEN** an admin changes that event back to `Public`
- **THEN** its image SHALL again be served to anonymous callers, still with no file
  movement
