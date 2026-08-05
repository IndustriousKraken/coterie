//! a57: event descriptions go through the SAME Markdown pipeline
//! announcements already use.
//!
//! What is pinned here:
//!
//! - `/public/events` carries a rendered `description_html` beside the raw
//!   `description`, holding only the safe subset (5.1, 5.2).
//! - For a members-only event that rendering derives from the PLACEHOLDER,
//!   never the withheld real description — the ordering guard for the
//!   sanitize-then-project rule (5.3).
//! - Rendering does not touch what is stored: the raw Markdown still
//!   round-trips to the admin edit form exactly as typed (5.4).
//! - The registration and class pages render emphasis as formatting rather
//!   than as literal asterisks (5.5).
//! - Both event forms carry the announcement editor's Markdown hint (5.6).
//! - There is still exactly ONE Markdown renderer in the tree (5.7) — the
//!   defect class is a second pipeline whose safe subset drifts.
//!
//! Run: cargo test --test event_description_markdown_test

use std::path::{Path, PathBuf};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use chrono::{Duration, Utc};
use coterie::{
    domain::{Event, EventSeries, EventType, EventVisibility},
    repository::{
        EventRepository, EventSeriesRepository, SqliteEventRepository, SqliteEventSeriesRepository,
    },
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool, make_member};

/// One description exercising everything the safe subset must keep and
/// everything it must drop. The bold line is the production case from the
/// proposal: it used to reach a social preview card as literal asterisks.
const RICH_DESCRIPTION: &str = "**Monthly Hack the Box and Training Night**\n\n\
    - bring a laptop\n\
    - bring a padlock\n\n\
    Sign up at [our site](https://example.com), not [here](javascript:alert(1)).\n\n\
    <script>alert(2)</script>\n\n\
    <a href=\"#\" onclick=\"steal()\">tap</a>\n\n\
    ![banner](https://example.com/x.png)";

/// The withheld description of the members-only event. Every word is
/// distinctive so "no fragment of it survives" is a real assertion.
const SECRET_DESCRIPTION: &str = "**Zeroday** briefing at the safehouse, back door unlocked";

async fn make_event(
    pool: &SqlitePool,
    description: &str,
    visibility: EventVisibility,
    guest_registration_enabled: bool,
) -> Event {
    let repo = SqliteEventRepository::new(pool.clone());
    let creator = make_member(pool).await;
    let now = Utc::now();
    repo.create(Event {
        id: Uuid::new_v4(),
        title: "Hack the Box Night".to_string(),
        description: description.to_string(),
        event_type: EventType::Meeting,
        event_type_id: None,
        visibility,
        start_time: now + Duration::days(7),
        end_time: None,
        timezone: "UTC".to_string(),
        location: Some("The Shop".to_string()),
        max_attendees: None,
        rsvp_required: true,
        member_price_cents: 0,
        guest_price_cents: 0,
        guest_registration_enabled,
        image_url: None,
        created_by: creator,
        created_at: now,
        updated_at: now,
        series_id: None,
        occurrence_index: None,
    })
    .await
    .expect("create event")
}

/// A publicly enrollable class: a series carrying one Public occurrence,
/// whose prototype description is what the class page renders.
async fn make_class(pool: &SqlitePool, description: &str) -> EventSeries {
    let series_repo = SqliteEventSeriesRepository::new(pool.clone());
    let event_repo = SqliteEventRepository::new(pool.clone());
    let creator = make_member(pool).await;
    let now = Utc::now();
    let series = series_repo
        .create(EventSeries {
            id: Uuid::new_v4(),
            rule_kind: "weekly_by_day".to_string(),
            rule_json: r#"{"kind":"weekly_by_day","interval":1,"weekdays":["tue"]}"#.to_string(),
            until_date: Some(now + Duration::days(30)),
            materialized_through: now + Duration::days(30),
            member_price_cents: 0,
            guest_price_cents: 0,
            guest_registration_enabled: true,
            max_enrollments: None,
            created_by: creator,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("create series");
    event_repo
        .create(Event {
            id: Uuid::new_v4(),
            title: "Intro to Lockpicking".to_string(),
            description: description.to_string(),
            event_type: EventType::Workshop,
            event_type_id: None,
            visibility: EventVisibility::Public,
            start_time: now + Duration::days(7),
            end_time: None,
            timezone: "UTC".to_string(),
            location: None,
            max_attendees: None,
            rsvp_required: true,
            member_price_cents: 0,
            guest_price_cents: 0,
            guest_registration_enabled: false,
            image_url: None,
            created_by: creator,
            created_at: now,
            updated_at: now,
            series_id: Some(series.id),
            occurrence_index: Some(1),
        })
        .await
        .expect("create occurrence");
    series
}

async fn public_events(pool: &SqlitePool) -> Vec<serde_json::Value> {
    let app = coterie::api::create_app(build_app_state(pool.clone()).await);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/public/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .expect("json array")
        .as_array()
        .expect("array response")
        .clone()
}

/// GET a web page, optionally carrying a session cookie.
async fn get_page(pool: &SqlitePool, uri: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let app: Router = coterie::web::create_web_routes(build_app_state(pool.clone()).await);
    let mut req = Request::builder().uri(uri);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let resp = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

// 5.1 — bold, a list, and an https link render to their safe HTML
// equivalents on the public feed.
#[tokio::test]
async fn public_event_description_html_renders_formatting() {
    let pool = fresh_pool().await;
    make_event(&pool, RICH_DESCRIPTION, EventVisibility::Public, false).await;

    let events = public_events(&pool).await;
    let html = events[0]["description_html"]
        .as_str()
        .expect("description_html present on every public event");

    assert!(
        html.contains("<strong>Monthly Hack the Box and Training Night</strong>"),
        "bold renders as <strong>, not literal asterisks: {html}"
    );
    assert!(html.contains("<ul>"), "the list renders as <ul>: {html}");
    assert!(
        html.contains("<li>bring a laptop</li>"),
        "list items render: {html}"
    );
    assert!(
        html.contains("href=\"https://example.com\""),
        "the https link is preserved: {html}"
    );
}

// 5.2 — nothing outside the safe subset survives.
#[tokio::test]
async fn public_event_description_html_drops_unsafe_constructs() {
    let pool = fresh_pool().await;
    make_event(&pool, RICH_DESCRIPTION, EventVisibility::Public, false).await;

    let events = public_events(&pool).await;
    let html = events[0]["description_html"].as_str().unwrap();

    assert!(!html.contains("<script"), "no live script element: {html}");
    assert!(!html.contains("<img"), "no image element: {html}");
    assert!(
        !html.contains("javascript:"),
        "no javascript: scheme: {html}"
    );
    // The `onclick` anchor never becomes a live tag: comrak escapes raw
    // HTML to inert text, so the handler exists only as characters on the
    // page, never as an attribute in the DOM.
    assert!(
        !html.contains("<a href=\"#\""),
        "the raw anchor must not become a live tag: {html}"
    );
    assert!(
        html.contains("&lt;a href=\"#\""),
        "…it renders as escaped literal text instead: {html}"
    );
}

// 5.3 — THE ORDERING GUARD. The members-only sanitizer replaces the
// description with a fixed placeholder before the projection is built;
// rendering the row's real description instead would publish exactly what
// the projection withheld. This fails if rendering is ever moved ahead of
// sanitization or sourced from the underlying row.
#[tokio::test]
async fn members_only_description_html_derives_from_the_placeholder() {
    let pool = fresh_pool().await;
    make_event(
        &pool,
        SECRET_DESCRIPTION,
        EventVisibility::MembersOnly,
        false,
    )
    .await;

    let events = public_events(&pool).await;
    let event = &events[0];
    assert_eq!(event["visibility"], "MembersOnly");

    let html = event["description_html"].as_str().unwrap();
    assert!(
        html.contains("This event is for members only."),
        "the rendered field is the rendered placeholder: {html}"
    );
    for fragment in ["Zeroday", "safehouse", "back door", "<strong>"] {
        assert!(
            !html.contains(fragment),
            "no fragment of the withheld description may reach the rendered \
             field — found `{fragment}` in: {html}"
        );
    }
}

// 5.4 — rendering adds a field, it does not rewrite one. The stored raw
// Markdown is byte-identical afterwards, on the feed AND in the textarea
// the admin edits it in.
#[tokio::test]
async fn raw_description_is_unchanged_and_round_trips_to_the_edit_form() {
    // No HTML metacharacters: the admin form escapes on output, and what
    // this test is about is the Markdown surviving untouched.
    const TYPED: &str = "**Monthly Hack the Box and Training Night**\n\n- bring a laptop";

    let pool = fresh_pool().await;
    let event = make_event(&pool, TYPED, EventVisibility::Public, false).await;

    // The stored row still holds exactly what was typed.
    let stored = SqliteEventRepository::new(pool.clone())
        .find_by_id(event.id)
        .await
        .unwrap()
        .expect("event still exists");
    assert_eq!(
        stored.description, TYPED,
        "rendering must not mutate storage"
    );

    // …and so does the raw field on the public feed.
    let events = public_events(&pool).await;
    assert_eq!(events[0]["description"], TYPED);

    // …and the admin edit form hands it back verbatim, not rendered.
    let cookie = admin_cookie(&pool).await;
    let (status, body) = get_page(
        &pool,
        &format!("/portal/admin/events/{}", event.id),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin event detail page");
    assert!(
        body.contains(TYPED),
        "the edit form round-trips the raw Markdown as typed"
    );
    assert!(
        !body.contains("<strong>Monthly Hack the Box and Training Night</strong>"),
        "the editor edits Markdown — the textarea is not a rendered preview"
    );
}

/// An Active admin plus the `session=` cookie for them.
async fn admin_cookie(pool: &SqlitePool) -> String {
    let admin = make_member(pool).await;
    sqlx::query("UPDATE members SET is_admin = 1, status = 'Active' WHERE id = ?")
        .bind(admin.to_string())
        .execute(pool)
        .await
        .unwrap();
    let state = build_app_state(pool.clone()).await;
    let (_session, token) = state
        .service_context
        .auth_service
        .create_session(admin, 24)
        .await
        .expect("create session");
    format!("session={token}")
}

// 5.5 — Coterie's own pages render emphasis as formatting.
#[tokio::test]
async fn registration_page_renders_emphasis_not_asterisks() {
    let pool = fresh_pool().await;
    let event = make_event(&pool, "**Bring a padlock**", EventVisibility::Public, true).await;

    let (status, body) = get_page(&pool, &format!("/events/{}/register", event.id), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("<strong>Bring a padlock</strong>"),
        "registration page renders emphasis: {body}"
    );
    assert!(
        !body.contains("**Bring a padlock**"),
        "asterisks must not reach the page as punctuation"
    );
}

#[tokio::test]
async fn class_page_renders_emphasis_not_asterisks() {
    let pool = fresh_pool().await;
    let series = make_class(&pool, "**Six Tuesdays**").await;

    let (status, body) = get_page(&pool, &format!("/classes/{}/register", series.id), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("<strong>Six Tuesdays</strong>"),
        "class page renders emphasis: {body}"
    );
    assert!(
        !body.contains("**Six Tuesdays**"),
        "asterisks must not reach the page as punctuation"
    );
}

// 2.3 — a consumer reads the endpoint's contract from the schema, so the
// added field belongs there exactly as the announcement's does.
#[test]
fn openapi_schema_documents_description_html() {
    use utoipa::OpenApi;

    let json = serde_json::to_value(coterie::api::docs::ApiDoc::openapi()).unwrap();
    for pointer in [
        "/components/schemas/PublicEvent/properties/description_html",
        "/components/schemas/PublicAnnouncement/properties/content_html",
    ] {
        assert!(
            json.pointer(pointer).is_some(),
            "the OpenAPI schema must document {pointer}",
        );
    }
}

fn repo_file(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

// 5.6 — both event forms carry the hint, in the announcement editor's own
// words. The wording is READ from the announcement editor rather than
// duplicated here, so the two cannot drift into implying different
// capabilities.
#[test]
fn both_event_forms_carry_the_announcement_editors_markdown_hint() {
    let announcement = repo_file("templates/admin/announcement_detail.html");
    let hint = announcement
        .lines()
        .find(|l| l.contains("Supports Markdown formatting"))
        .expect("the announcement editor states its Markdown support")
        .trim()
        .to_string();

    for form in [
        "templates/admin/event_new.html",
        "templates/admin/event_detail.html",
    ] {
        assert!(
            repo_file(form).lines().any(|l| l.trim() == hint),
            "{form} must carry the announcement editor's hint verbatim: {hint}"
        );
    }

    // The create form also primes the empty box, as the announcement
    // create form does before anything is typed.
    assert!(
        repo_file("templates/admin/event_new.html").contains("Markdown formatting is supported."),
        "the create form's placeholder says so too"
    );
}

// 5.7 — one renderer, one safe subset. A second comrak/ammonia call site
// is a second set of sanitization decisions to keep aligned, which is the
// defect this capability exists to prevent.
#[test]
fn only_one_markdown_renderer_exists() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_files(&src) {
        let rel = path
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel == "util/markdown.rs" {
            continue;
        }
        for (i, line) in repo_file(&format!("src/{rel}")).lines().enumerate() {
            if line.contains("comrak") || line.contains("ammonia") {
                offenders.push(format!("src/{rel}:{}: {}", i + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "Markdown rendering and HTML sanitization live in src/util/markdown.rs \
         and nowhere else — call `render_markdown` instead of standing up a \
         second pipeline:\n{}",
        offenders.join("\n"),
    );
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).expect("read_dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out
}
