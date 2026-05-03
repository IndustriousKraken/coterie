## ADDED Requirements

### Requirement: UniFi integration gates network access by member state

The UniFi integration SHALL react to integration events that affect a member's network-access entitlement and call the UniFi controller outbound to add or remove the member from the appropriate group / VLAN.

#### Scenario: Activation grants network access

- **WHEN** a member transitions to Active
- **THEN** the integration SHALL call UniFi to grant their configured access

#### Scenario: Suspension or expiry revokes access

- **WHEN** a member transitions to Suspended or Expired
- **THEN** the integration SHALL revoke UniFi access

### Requirement: Failures are surfaced, not silent

If the UniFi call fails, the originating member-state change SHALL NOT be rolled back. The failure SHALL be logged and reflected in admin-visible failure surfaces (logs, dashboards) so an operator can retry manually.

#### Scenario: Failed revoke does not block suspension

- **WHEN** an admin suspends a member and the UniFi call fails
- **THEN** the member's status SHALL still transition to Suspended; the failure SHALL be visible to admins for follow-up
