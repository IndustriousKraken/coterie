## 1. Extend domain and repository

- [ ] 1.1 In `src/domain/member.rs::CreateMemberRequest`, add five `Option`-wrapped optional fields:
  ```rust
  pub dues_paid_until: Option<DateTime<Utc>>,
  pub stripe_customer_id: Option<String>,
  pub stripe_subscription_id: Option<String>,
  pub joined_at: Option<DateTime<Utc>>,
  pub email_verified_at: Option<DateTime<Utc>>,
  ```
- [ ] 1.2 Update `SqliteMemberRepository::create` to honor the new fields when `Some`. The `INSERT` query SHALL substitute the supplied value or the current default per the existing schema columns. `joined_at` defaults to `NOW()` when `None`; the others default to `NULL`.
- [ ] 1.3 Update any existing in-tree construction of `CreateMemberRequest` (admin "create member" form, public signup, test fixtures) to supply `..Default::default()` for the new fields or to construct them as `None`. The admin form does not expose these fields to the operator in v1 — it just passes `None` for all five.

## 2. Extend ImportRow and bulk_import

- [ ] 2.1 In `src/service/member_service.rs::ImportRow`, add the same five `Option` fields.
- [ ] 2.2 In `MemberService::bulk_import`, thread the new fields from `ImportRow` into `CreateMemberRequest` for each row.
- [ ] 2.3 Add the inconsistency check: if `row.stripe_subscription_id.is_some() && row.stripe_customer_id.is_none()`, fail the row with reason `"Stripe subscription_id present without customer_id"`. Add this BEFORE the `repo.create` call so no member is created for the bad row.
- [ ] 2.4 After `repo.create` succeeds for a row, if `row.stripe_subscription_id.is_some()`, call `repo.set_billing_mode(new_member.id, BillingMode::StripeSubscription, Some(&sub_id))` to flip the mode. This is the inference per design D2.
- [ ] 2.5 If `row.email_verified_at.is_some()`, suppress the verification email send for that row. (The existing `bulk_import` does NOT send verification emails today — members are created Pending and the operator activates them via the admin UI. Verify this in the implementation; if the email-send logic IS in the bulk path, gate it on `email_verified_at.is_none()`. If it's not, no change needed.)

## 3. Extend the CSV parser

- [ ] 3.1 In `src/web/portal/admin/members/bulk.rs::parse_import_csv`, recognize the five new optional column names in the header. Header order doesn't matter (the existing parser indexes by column name, not position — confirm during implementation).
- [ ] 3.2 For each timestamp column (`dues_paid_until`, `joined_at`, `email_verified_at`), attempt `DateTime::parse_from_rfc3339(cell)`. If that fails AND the cell looks like `YYYY-MM-DD`, attempt `NaiveDate::parse_from_str(cell, "%Y-%m-%d")` and promote to `DateTime<Utc>` at midnight UTC. If both fail, the row gets a parse failure with reason `"Could not parse <field>: '<cell value>'"`.
- [ ] 3.3 For `stripe_customer_id` and `stripe_subscription_id`, just trim whitespace and pass through as `String`. Empty after trim → `None`.

## 4. Update the format-reminder UI

- [ ] 4.1 In `templates/admin/member_import.html`, update the format-reminder block (the `<details>` element) to list the new optional columns alongside the existing ones. Group them as "Billing-migration fields" so operators understand when they're relevant.
- [ ] 4.2 Add a brief inline hint: "Provide `stripe_customer_id` + `stripe_subscription_id` together when importing members from an existing Stripe-billed system. Coterie will observe the existing subscription until the member organically migrates."

## 5. Tests

- [ ] 5.1 Add an integration test `import_with_stripe_subscription_sets_mode`: seed a member-type, build a CSV with one row containing `stripe_customer_id` and `stripe_subscription_id`, run the import, assert the created member has `billing_mode = StripeSubscription` and both IDs are persisted.
- [ ] 5.2 Add `import_with_customer_only_stays_manual`: similar setup but only `stripe_customer_id` is supplied (no subscription). Assert `billing_mode = Manual` and `stripe_customer_id` is persisted.
- [ ] 5.3 Add `import_subscription_without_customer_fails_row`: build a CSV with `stripe_subscription_id` but empty `stripe_customer_id`. Assert the import summary reports this row as failed with the documented reason; no member is created.
- [ ] 5.4 Add `import_with_dues_paid_until_persists_date`: build a CSV with `dues_paid_until = 2027-01-15`. Assert the created member's `dues_paid_until` matches.
- [ ] 5.5 Add `import_malformed_timestamp_fails_row`: build a CSV with `joined_at = not-a-date`. Assert the row fails with the documented parse-error reason; subsequent rows in the same CSV succeed.
- [ ] 5.6 Add `import_email_verified_at_skips_verification_email`: build a CSV with `email_verified_at` set. Assert the row succeeds, the member's `email_verified_at` is populated, and no verification email was queued (use the test EmailSender fake's call log).
- [ ] 5.7 Confirm existing import tests still pass without modification (they should — all new fields are optional).

## 6. Validate

- [ ] 6.1 `cargo build --all-targets --features test-utils` — clean.
- [ ] 6.2 `cargo test --features test-utils` — full suite passes including the new tests.
