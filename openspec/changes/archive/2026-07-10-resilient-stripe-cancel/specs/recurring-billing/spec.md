# recurring-billing Specification (delta)

## ADDED Requirements

### Requirement: Subscription cancel tolerates an unparseable success response

The subscription-cancel gateway call SHALL treat an inability to parse the
Stripe response as success when the request itself carried no body, because
in that case the failure is on the response and Stripe has already processed
the cancellation. The gateway SHALL distinguish a response-parse error from a
genuine Stripe API error (a returned error status) and from a transport error
(network or timeout), and SHALL return failure only for the latter two. A
warning SHALL be logged when a cancel is accepted on a parse-tolerated basis,
noting that the `customer.subscription.deleted` webhook will reconcile the
member's state.

#### Scenario: A successful cancel with an unparseable response is not a failure

- **WHEN** the cancel request is delivered to Stripe and the subscription is
  cancelled, but the client cannot deserialize the returned object
- **THEN** the cancel SHALL be reported as successful, no error SHALL be shown
  to the member, and a warning SHALL be logged

#### Scenario: A genuine API error is still a failure

- **WHEN** Stripe returns an error status for the cancel (for example the
  subscription does not exist), or the request fails on the network
- **THEN** the cancel SHALL be reported as a failure and surfaced to the caller

### Requirement: A parse-tolerated cancel does not roll back local state

The auto-renew flows SHALL keep the member's intended post-cancel billing
mode in place rather than rolling it back when a cancel is treated as
successful. Rolling back on a false failure previously raced the
subscription-deleted webhook and left the member in an inconsistent billing
mode.

#### Scenario: No rollback race with the webhook

- **WHEN** a member cancels and the gateway tolerates the unparseable response
  as success
- **THEN** the member's billing mode SHALL reflect the intended post-cancel
  state without being rolled back, and SHALL agree with the state the
  `customer.subscription.deleted` webhook sets

### Requirement: Cancel resilience is verifiable without live Stripe

The behavior SHALL be testable offline through the existing fake gateway seam
(feature `test-utils`). The fake SHALL be able to simulate a response-parse
error on the cancel call, and a test SHALL assert the caller treats it as
success with no rollback, while a simulated genuine API error SHALL still
surface as a failure.

#### Scenario: Fake gateway reproduces the tolerated-parse case

- **WHEN** the fake gateway is configured to return a response-parse error on
  cancel
- **THEN** the cancel flow SHALL succeed without rollback in a unit test, with
  no real Stripe credentials involved
