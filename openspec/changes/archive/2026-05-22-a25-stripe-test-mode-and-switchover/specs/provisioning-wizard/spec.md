## ADDED Requirements

### Requirement: Wizard offers a test-mode-or-live-mode choice

The wizard SHALL prompt the operator to choose between test mode and live mode when configuring Stripe. The prompt SHALL be presented if and only if Stripe is being enabled. Default selection: **live mode** (matching the `a24` wizard's original behavior).

If test mode is selected:
- The wizard SHALL collect test-mode Stripe credentials (`pk_test_*`, `sk_test_*`, test webhook signing secret `whsec_…`).
- The wizard SHALL configure `.env` with `DATABASE_URL` pointing at `coterie-test.db`.
- The wizard MAY (operator's choice) ALSO collect live-mode credentials and stash them in `/opt/coterie/.env.live` for the future switchover.
- After the wizard completes, a verification checklist SHALL be printed describing the manual flows to test and the command to run when ready to switch to live mode.

If live mode is selected:
- Wizard behavior is identical to the `a24` baseline (collects live credentials, `coterie.db` is the database).

#### Scenario: Test mode selected, switchover guidance printed

- **WHEN** the wizard runs with test mode selected
- **THEN** the final output SHALL include a verification checklist (suggested flows + how to run each) and the command line to invoke `switch-stripe-to-live.sh` when ready

#### Scenario: Live mode selected behaves identically to a24

- **WHEN** the wizard runs with live mode selected
- **THEN** the resulting Coterie instance SHALL match the `a24` baseline behavior — `.env` configured with live credentials, `coterie.db` as the database, no `coterie-test.db` or `.env.live` artifacts on disk

#### Scenario: Env-var override skips the test-or-live prompt

- **WHEN** `COTERIE_PROVISION_STRIPE_MODE=test` or `=live` is set in the environment
- **THEN** the wizard SHALL skip the prompt and use the specified mode
