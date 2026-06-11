/// The verification checklist printed at the end of the wizard when
/// test mode is selected. See openspec change a25 design.md D8.
pub const TEST_MODE_CHECKLIST: &str =
    "Coterie is running in Stripe TEST mode with separate test database
(/var/lib/coterie/coterie-test.db). Use this time to verify Stripe
wiring before switching to live.

Test card to use: 4242 4242 4242 4242, any future expiry, any 3-digit
CVC, any ZIP.

Suggested verification steps:

  [ ] Sign up a test member via your public site or directly via
      Coterie's signup form (if exposed).
  [ ] Make a test donation through /portal/donate (logged in as
      admin) or via the public donate flow.
  [ ] Confirm each test charge appears in your Stripe dashboard's
      TEST MODE payments view.
  [ ] Confirm `journalctl -u coterie` shows the webhook events
      arriving cleanly (look for \"Webhook event received\").
  [ ] Confirm the receipt email arrived at the address you used.

When satisfied, switch to live mode:

  sudo coterie-provision switch-stripe-to-live

This will: stop Coterie, archive coterie-test.db, create a fresh
coterie.db, copy your admin row across, prompt for (or load) your
live Stripe credentials, rewrite .env, and start Coterie back up.";
