## 1. Add single-use and expiry tests for EmailTokenService::consume

- [ ] 1.1 `consume_redeems_exactly_once` — `create()` then two `consume()`s;
  assert first returns `Some(ConsumedToken { member_id })`, second returns
  `None`.
- [ ] 1.2 `consume_rejects_expired_token` — insert a row directly via
  `sqlx::query` with `expires_at = Utc::now() - Duration::hours(1)`; assert
  `consume(plaintext)` returns `None` and the row's `consumed_at` is still
  NULL afterward (no spurious update).
- [ ] 1.3 `consume_rejects_unknown_token` — assert
  `consume("not-a-real-token")` returns `Ok(None)`.

## 2. Add cross-purpose-isolation tests

- [ ] 2.1 `password_reset_token_does_not_consume_at_verification_service` —
  mint a token via `password_reset(pool)`; assert
  `verification(pool).consume(token).unwrap().is_none()`.
- [ ] 2.2 `verification_token_does_not_consume_at_password_reset_service` —
  mirror of 2.1.

## 3. Add invalidate_for_member and cleanup_expired tests

- [ ] 3.1 `invalidate_for_member_marks_outstanding_consumed` — mint two
  tokens for the same member via `password_reset`; call
  `invalidate_for_member`; assert both subsequent `consume` calls return
  `None`.
- [ ] 3.2 `cleanup_expired_deletes_only_expired_rows` — mint one fresh
  token, insert one already-expired row directly; assert
  `cleanup_expired().await.unwrap() == 1` and that the fresh token still
  redeems via `consume`.

## 4. Lock in storage invariant

- [ ] 4.1 `created_token_is_sha256_of_plaintext` — `create()` returns
  `CreatedToken { token, .. }`; read the row's `token_hash` directly via
  sqlx and assert it equals `hex::encode(Sha256::digest(token.as_bytes()))`.
