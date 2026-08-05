# deployment-updates Specification Delta

## ADDED Requirements

### Requirement: Update reports host state the release expects but does not find

`update` SHALL check, after placing files, whether the host is in the state the
installed release expects, and SHALL name each discrepancy together with the
command that resolves it.

Refreshing a unit file on a host that has never enabled it leaves a capability
present on disk and absent in operation, and nothing in the deployment is
otherwise in a position to notice. The backup units shipped in a release and were
never enabled on an instance provisioned before the wizard installed them; no
backup ran for months while `VERSION` reported a current release and the scripts
sat beside it looking installed. The gap was not that a manual step existed — it
was that no component compared what the release assumed against what the host
had.

The check SHALL cover, at minimum, whether units the release ships are enabled on
the host, and SHALL be structured so further checks can be added as releases ship
more.

`update` SHALL NOT resolve any discrepancy it reports. It SHALL NOT enable,
start, or reload anything, and SHALL NOT write outside the install directory. An
update that enables units can start a service an operator deliberately left off,
on a host whose intent it cannot see; reporting carries none of that risk and
nearly all of the value, because the failure being addressed is that nobody knew,
not that someone declined.

A reported discrepancy SHALL NOT fail the update. `update` SHALL exit zero, the
update itself having succeeded. An advisory finding that produces a non-zero exit
teaches operators to stop reading exit codes.

An update against a host with no discrepancies SHALL report none. The report's
presence is the signal, which holds only if it is absent whenever there is
nothing to say.

The check SHALL route its host inspection through the same abstractions the rest
of the update flow uses, so it is exercisable by fakes with no real process or
filesystem access.

#### Scenario: A shipped-but-unenabled unit is reported

- **WHEN** an update completes on a host where a unit the release ships is not
  enabled
- **THEN** the output SHALL name that unit and the command that enables it

#### Scenario: A conformant host reports nothing

- **WHEN** an update completes on a host that is in the state the release expects
- **THEN** no discrepancy SHALL be reported

#### Scenario: A discrepancy does not fail the update

- **WHEN** one or more discrepancies are reported
- **THEN** `update` SHALL exit zero

#### Scenario: Update does not resolve the discrepancy

- **WHEN** a unit is reported as not enabled
- **THEN** `update` SHALL NOT enable or start it, and the host's unit state SHALL
  be unchanged

#### Scenario: The check is driven by fakes in tests

- **WHEN** the conformance check runs under test
- **THEN** it SHALL obtain host state through the same abstractions the update
  flow already uses, with no real process execution
