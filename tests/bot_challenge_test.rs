//! Sanity tests for the bot-challenge gate on `/public/signup` and
//! `/public/donate`. The HTTP-level integration test (spinning up the
//! full router) lives in `tests/csrf_coverage_test.rs` and exercises
//! `DisabledVerifier` because that's the production default for tests.
//! These tests exercise the verifier surface directly with the
//! `FakeVerifier` from `test_utils`.

use coterie::api::middleware::bot_challenge::{
    BotChallengeVerifier, DisabledVerifier, VerifyError,
    test_utils::FakeVerifier,
};

#[tokio::test]
async fn disabled_verifier_passes_with_no_token() {
    let v = DisabledVerifier;
    let result = v.verify("public/signup", None, None).await;
    assert!(result.is_ok(), "DisabledVerifier should accept missing tokens");
}

#[tokio::test]
async fn disabled_verifier_passes_with_any_token() {
    let v = DisabledVerifier;
    let result = v.verify("public/signup", Some("anything"), None).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn fake_verifier_rejects_missing_token() {
    let v = FakeVerifier::new(|_t| Ok(()));
    let err = v.verify("public/donate", None, None).await.unwrap_err();
    assert!(matches!(err, VerifyError::Missing));
    assert_eq!(v.call_count(), 1);
}

#[tokio::test]
async fn fake_verifier_routes_through_decision_closure() {
    let v = FakeVerifier::new(|t| {
        if t == "good-token" {
            Ok(())
        } else {
            Err(VerifyError::Invalid { provider_codes: vec!["bad".into()] })
        }
    });

    assert!(v.verify("public/signup", Some("good-token"), None).await.is_ok());
    let err = v.verify("public/signup", Some("anything-else"), None).await.unwrap_err();
    assert!(matches!(err, VerifyError::Invalid { .. }));
    assert_eq!(v.call_count(), 2);
}
