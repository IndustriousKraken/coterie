//! Smoke test for the public OpenAPI specification.
//!
//! Verifies that:
//!   1. The `ApiDoc` struct compiles + serializes to JSON without panicking
//!      (catches utoipa::path macro mistakes — bad refs, missing types,
//!      schemas referenced but not registered, etc.).
//!   2. Every endpoint we intentionally documented is present in the spec.
//!   3. Every component schema we registered is present.
//!
//! Run: cargo test --test openapi_test

use coterie::api::docs::ApiDoc;
use utoipa::OpenApi;

#[test]
fn openapi_spec_compiles_and_serializes() {
    let doc = ApiDoc::openapi();
    let json = serde_json::to_value(&doc).expect("openapi spec serializes");
    assert!(json.get("openapi").is_some(), "spec has openapi version field");
    assert!(json.get("info").is_some(), "spec has info block");
    assert!(json.get("paths").is_some(), "spec has paths block");
}

#[test]
fn openapi_spec_documents_all_public_endpoints() {
    let doc = ApiDoc::openapi();
    let json = serde_json::to_value(&doc).unwrap();
    let paths = json.get("paths").and_then(|p| p.as_object()).expect("paths object");

    // (path, method) pairs we intentionally document.
    let expected: &[(&str, &str)] = &[
        ("/", "get"),
        ("/health", "get"),
        ("/api", "get"),
        ("/public/signup", "post"),
        ("/public/events", "get"),
        ("/public/events/private-count", "get"),
        ("/public/announcements", "get"),
        ("/public/announcements/private-count", "get"),
        ("/public/feed/rss", "get"),
        ("/public/feed/calendar", "get"),
        ("/public/donate", "post"),
    ];

    for (path, method) in expected {
        let entry = paths.get(*path).unwrap_or_else(|| {
            panic!("spec is missing path {}", path);
        });
        assert!(
            entry.get(*method).is_some(),
            "spec is missing {} on {}",
            method.to_uppercase(),
            path,
        );
    }
}

#[test]
fn openapi_spec_registers_all_dto_schemas() {
    let doc = ApiDoc::openapi();
    let json = serde_json::to_value(&doc).unwrap();
    let schemas = json
        .pointer("/components/schemas")
        .and_then(|s| s.as_object())
        .expect("components.schemas object");

    let expected_schemas: &[&str] = &[
        "ApiInfo",
        "HealthStatus",
        "SignupRequest",
        "SignupResponse",
        "PrivateEventCount",
        "PublicDonateRequest",
        "PublicDonateResponse",
        "PrivateAnnouncementCount",
        "Event",
        "EventType",
        "EventVisibility",
        "Announcement",
        "AnnouncementType",
        "MemberStatus",
    ];

    for name in expected_schemas {
        assert!(
            schemas.contains_key(*name),
            "spec is missing component schema {}",
            name,
        );
    }
}
