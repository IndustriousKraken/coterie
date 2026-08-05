# public-site-notifications Specification Delta

## ADDED Requirements

### Requirement: Coterie can notify a configured public site of content changes

The system SHALL support notifying one configured companion public site when
public events or announcements change, by posting a change notification to a
configured endpoint.

Configuration SHALL consist of an endpoint URL and a shared secret. When no
endpoint is configured the feature SHALL be entirely inert: nothing is sent, no
error is raised, and no existing behavior changes. Every deployment that does not
have a companion site SHALL be unaffected by this capability's existence.

The shared secret SHALL be stored as a sensitive setting and SHALL NOT be
rendered back to an admin in readable form or written to logs, in line with how
other integration credentials are handled.

A notification SHALL be authenticated so the receiver can distinguish it from an
arbitrary internet request. The receiving site acts on these messages by
publishing or withdrawing content, so an unauthenticated endpoint would let any
caller drive that.

A notification SHALL carry the item's kind, its identifier, and what happened, and
SHALL carry no item content at all. A receiver that needs content SHALL read it
from the public API, which already applies the organization's visibility rules.

Carrying no content is what makes the disclosure question unaskable rather than
merely answered carefully. If the payload held content, correctness would depend
on the receiver declining to publish what it was given — a rule that holds only
while every receiver, present and future, implements it right, and whose failure
is silent. With an identifier alone, a receiver can never render anything it was
not already entitled to fetch, no matter what it does with the message.

#### Scenario: No endpoint configured means no behavior change

- **WHEN** no public-site endpoint is configured and a public event is edited
- **THEN** no notification SHALL be attempted and the edit SHALL behave exactly as
  it does today

#### Scenario: A notification is authenticated

- **WHEN** a notification is sent
- **THEN** it SHALL carry authentication derived from the configured shared secret

#### Scenario: A notification carries no item content

- **WHEN** any notification is sent, for an item of any visibility
- **THEN** its payload SHALL contain the item's kind, identifier, and what
  happened, and SHALL NOT contain the item's title, description, location, or
  image

#### Scenario: The secret is not disclosed

- **WHEN** an admin views the settings page or the logs are inspected
- **THEN** the configured shared secret SHALL NOT appear in readable form

### Requirement: Withdrawing public content is confirmed before the admin is done

An action that withdraws an item from public view SHALL notify the configured
public site **synchronously** and SHALL report that notification's outcome in the
response the admin receives. Withdrawal here means deleting an item, or changing
its visibility or publication state so it leaves the public API.

This SHALL NOT be delivered through the fire-and-forget integration fan-out. That
mechanism is specified to not block the originating call and to log failures
rather than surface them, which is correct for notifications whose loss is
recoverable and wrong for this one: a withdrawal that fails to arrive leaves
content public with nobody aware. A best-effort channel cannot carry a control
whose failure is a disclosure.

Delivering this synchronously SHALL NOT require a durable queue, retry
infrastructure, or an outbox. The property that makes those unnecessary is that
an administrator is present and waiting at the moment of withdrawal, so a failure
can be reported to someone able to act on it immediately. Building durable
delivery to serve a case that already has a human in the loop would add an
operational component for no gain.

A failed notification SHALL NOT roll back the withdrawal. The item is withdrawn
in Coterie regardless; what failed is telling the other system, and reverting the
withdrawal would make the two more inconsistent rather than less.

The reported outcome SHALL distinguish success from failure plainly enough that
an admin knows whether the public site is up to date, and SHALL point to the
means of retrying.

The notification attempt SHALL be bounded in time so a slow or unresponsive
endpoint delays the admin's response by a bounded amount rather than hanging it.

#### Scenario: A successful withdrawal reports that the public site is updated

- **WHEN** an admin withdraws a public item and the notification succeeds
- **THEN** the response SHALL indicate the public site was updated

#### Scenario: A failed withdrawal tells the admin

- **WHEN** an admin withdraws a public item and the notification fails
- **THEN** the response SHALL indicate the public site was NOT updated and SHALL
  indicate how to retry

#### Scenario: A failed notification does not undo the withdrawal

- **WHEN** the notification fails
- **THEN** the item SHALL remain withdrawn in Coterie

#### Scenario: An unresponsive endpoint does not hang the admin

- **WHEN** the configured endpoint does not respond
- **THEN** the attempt SHALL end within its bound and the admin SHALL receive a
  response reporting the failure

### Requirement: An admin can resend an item's current state on demand

The event and announcement admin surfaces SHALL each offer a per-item control
that resends that item's current state to the configured public site and reports
the outcome.

This control is the retry path for a notification that failed, and the recovery
path when the automatic notification is broken for any reason — misconfiguration,
a receiver that was down, a change made before the endpoint was configured. It is
the one part of this capability that does not depend on any other part working.

It SHALL be per item rather than a global rebuild. An admin using it is asking
about one item and needs an answer about that item; a bulk operation reports
success without establishing that the thing they care about is fixed.

The control SHALL be available regardless of whether the item is currently public,
since resending the state of a withdrawn item is exactly how an admin repairs a
withdrawal the public site missed.

The control SHALL be present only when an endpoint is configured.

#### Scenario: Resending reports success

- **WHEN** an admin uses the resend control and the notification succeeds
- **THEN** the outcome SHALL be reported as successful

#### Scenario: Resending repairs a missed withdrawal

- **WHEN** an item was withdrawn while the public site was unreachable, and an
  admin later uses the resend control for that item
- **THEN** a withdrawal notification SHALL be sent for it

#### Scenario: The control is absent when unconfigured

- **WHEN** no public-site endpoint is configured
- **THEN** no resend control SHALL be shown
