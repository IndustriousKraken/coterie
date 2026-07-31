//! Storage separation for private uploads (a49).
//!
//! The claim under test is structural, not procedural: `/uploads/:filename`
//! serves the public root and nothing else, so a submission attachment is
//! unreachable there no matter what the `submissions` table says — or stops
//! saying. Alongside it, the public-image predicate is an allow-list: a file
//! nothing affirms is public is refused rather than served.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use chrono::{Duration, Utc};
use coterie::{
    api::state::AppState,
    domain::{
        Announcement, AnnouncementType, CreateMemberRequest, Event, EventType, EventVisibility,
        MemberStatus, Submission, SubmissionStatus, SubmissionVisibility, UpdateMemberRequest,
    },
    repository::{EventRepository, SqliteEventRepository},
    web::uploads::{migrate_attachments_to_private_root, save_uploaded_document},
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

mod common;
use common::{build_app_state, fresh_pool};

/// Smallest byte string `save_uploaded_file` accepts as a real PNG.
const PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xDE, 0xAD];

async fn enable_submissions(state: &AppState) {
    sqlx::query("UPDATE app_settings SET value = 'true' WHERE key = 'submissions.enabled'")
        .execute(&state.service_context.db_pool)
        .await
        .expect("enable submissions toggle");
}

async fn make_session(state: &AppState, is_admin: bool) -> (Uuid, String) {
    let suffix = Uuid::new_v4();
    let member = state
        .service_context
        .member_repo
        .create(CreateMemberRequest {
            email: format!("u-{}@example.com", suffix),
            username: format!("u_{}", suffix.simple()),
            full_name: "Test Member".into(),
            password: "p4ssword_long_enough".into(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member");
    state
        .service_context
        .member_repo
        .update(
            member.id,
            UpdateMemberRequest {
                status: Some(MemberStatus::Active),
                ..Default::default()
            },
        )
        .await
        .expect("activate member");
    if is_admin {
        state
            .service_context
            .member_repo
            .set_admin(member.id, true)
            .await
            .expect("set admin");
    }
    let (_, token) = state
        .service_context
        .auth_service
        .create_session(member.id, 24)
        .await
        .expect("create session");
    (member.id, token)
}

/// Store a real PDF in the private root and hang a submission row off it.
/// Returns (submission id, bare filename).
async fn attachment_submission(
    state: &AppState,
    owner: Uuid,
    status: SubmissionStatus,
) -> (Uuid, String) {
    let stored = save_uploaded_document(
        &state.settings.server.private_uploads_path(),
        b"%PDF-1.4 secret",
    )
    .await
    .expect("save pdf");
    let filename = stored
        .strip_prefix("private-uploads/")
        .expect("attachments are stored under the private prefix")
        .to_string();

    let now = Utc::now();
    let id = state
        .service_context
        .submission_repo
        .create(Submission {
            id: Uuid::new_v4(),
            submitter_member_id: owner,
            title: "Private".to_string(),
            abstract_text: "Body".to_string(),
            visibility_requested: SubmissionVisibility::Members,
            attachment_path: Some(stored),
            preferred_start: None,
            timezone: "UTC".to_string(),
            duration_minutes: None,
            status,
            reviewer_note: None,
            decided_by: None,
            event_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("insert submission")
        .id;
    (id, filename)
}

/// Store a real PNG in the public root and attach it to an event.
async fn event_with_image(
    state: &AppState,
    pool: &SqlitePool,
    visibility: EventVisibility,
) -> (Event, String) {
    let stored = coterie::web::uploads::save_uploaded_file(
        &state.settings.server.uploads_path(),
        "flyer.png",
        PNG,
    )
    .await
    .expect("save png");
    let filename = stored.strip_prefix("uploads/").unwrap().to_string();
    let creator = make_session(state, false).await.0;
    let now = Utc::now();
    let event = SqliteEventRepository::new(pool.clone())
        .create(Event {
            id: Uuid::new_v4(),
            title: "Lockpicking 101".to_string(),
            description: "Bring a padlock".to_string(),
            event_type: EventType::Workshop,
            event_type_id: None,
            visibility,
            start_time: now + Duration::days(7),
            end_time: None,
            timezone: "UTC".to_string(),
            location: None,
            max_attendees: None,
            rsvp_required: false,
            member_price_cents: 0,
            guest_price_cents: 0,
            guest_registration_enabled: false,
            image_url: Some(stored),
            created_by: creator,
            created_at: now,
            updated_at: now,
            series_id: None,
            occurrence_index: None,
        })
        .await
        .expect("create event");
    (event, filename)
}

/// Store a real PNG in the public root and attach it to an announcement.
/// Returns the bare filename.
async fn announcement_with_image(state: &AppState, is_public: bool) -> String {
    let stored = coterie::web::uploads::save_uploaded_file(
        &state.settings.server.uploads_path(),
        "banner.png",
        PNG,
    )
    .await
    .expect("save png");
    let filename = stored.strip_prefix("uploads/").unwrap().to_string();
    let author = make_session(state, true).await.0;
    let now = Utc::now();
    state
        .service_context
        .announcement_repo
        .create(Announcement {
            id: Uuid::new_v4(),
            title: "Notice".to_string(),
            content: "Body".to_string(),
            announcement_type: AnnouncementType::General,
            announcement_type_id: None,
            is_public,
            featured: false,
            image_url: Some(stored),
            published_at: Some(now),
            scheduled_publish_at: None,
            scheduled_publish_timezone: "UTC".to_string(),
            created_by: author,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("create announcement");
    filename
}

fn get(uri: &str, session: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(t) = session {
        b = b.header(header::COOKIE, format!("session={}", t));
    }
    b.body(Body::empty()).unwrap()
}

async fn status_of(app: &Router, uri: &str, session: Option<&str>) -> StatusCode {
    app.clone()
        .oneshot(get(uri, session))
        .await
        .unwrap()
        .status()
}

/// The public route must not hand back the file — to an anonymous caller
/// OR to a signed-in one, who clears the session gate and still gets
/// nothing because the file is not in the root being served.
async fn assert_not_served_publicly(app: &Router, filename: &str, session: &str) {
    let uri = format!("/uploads/{}", filename);
    let anon = status_of(app, &uri, None).await;
    assert_ne!(anon, StatusCode::OK, "anonymous caller was served {}", uri);
    let member = status_of(app, &uri, Some(session)).await;
    assert_eq!(
        member,
        StatusCode::NOT_FOUND,
        "an authenticated caller reached {} — the file is in the public root",
        uri
    );
}

// --- 5.1 / 5.2 / 5.3 Attachments are unreachable from the public route ---

#[tokio::test]
async fn attachment_is_not_served_by_the_public_route() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    let (owner, session) = make_session(&state, false).await;
    let (_, filename) = attachment_submission(&state, owner, SubmissionStatus::Submitted).await;
    let app = coterie::web::create_web_routes(state.clone());

    // 5.1 — with the row present.
    assert_not_served_publicly(&app, &filename, &session).await;
}

#[tokio::test]
async fn attachment_is_not_served_after_its_row_is_deleted() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (owner, session) = make_session(&state, false).await;
    let (id, filename) = attachment_submission(&state, owner, SubmissionStatus::Withdrawn).await;
    let app = coterie::web::create_web_routes(state.clone());

    // 5.2 — the regression this change exists for. Under the old deny-list
    // the vanished row was what published the file.
    let deleted = sqlx::query("DELETE FROM submissions WHERE id = ?")
        .bind(id.to_string())
        .execute(&pool)
        .await
        .expect("delete submission row");
    assert_eq!(deleted.rows_affected(), 1);
    assert_not_served_publicly(&app, &filename, &session).await;
}

#[tokio::test]
async fn deleting_a_member_does_not_publish_their_attachment() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (owner, _) = make_session(&state, false).await;
    let (_, other_session) = make_session(&state, false).await;
    let (id, filename) = attachment_submission(&state, owner, SubmissionStatus::Submitted).await;
    let app = coterie::web::create_web_routes(state.clone());

    // 5.3 — submitter_member_id is ON DELETE CASCADE, so removing the
    // member removes the row that used to be the file's only protection.
    sqlx::query("DELETE FROM members WHERE id = ?")
        .bind(owner.to_string())
        .execute(&pool)
        .await
        .expect("delete member");
    assert!(
        state
            .service_context
            .submission_repo
            .find_by_id(id)
            .await
            .unwrap()
            .is_none(),
        "the submission should have cascaded away with its member"
    );
    assert_not_served_publicly(&app, &filename, &other_session).await;
}

// --- 5.4 The gated route still works ------------------------------------

#[tokio::test]
async fn gated_route_serves_the_attachment_to_owner_and_admin() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool).await;
    enable_submissions(&state).await;
    let (owner, owner_session) = make_session(&state, false).await;
    let (_, admin_session) = make_session(&state, true).await;
    let (id, _) = attachment_submission(&state, owner, SubmissionStatus::Submitted).await;
    let app = coterie::web::create_web_routes(state.clone());

    for session in [&owner_session, &admin_session] {
        let resp = app
            .clone()
            .oneshot(get(
                &format!("/portal/submissions/{}/attachment", id),
                Some(session),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let headers = resp.headers();
        assert!(headers
            .get(header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("attachment")));
        assert_eq!(
            headers
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .and_then(|v| v.to_str().ok()),
            Some("nosniff")
        );
    }
}

// --- 5.5 / 5.6 Public images still work, without an attachment lookup ----

#[tokio::test]
async fn public_event_image_is_still_served_anonymously() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (_, filename) = event_with_image(&state, &pool, EventVisibility::Public).await;
    let app = coterie::web::create_web_routes(state.clone());

    assert_eq!(
        status_of(&app, &format!("/uploads/{}", filename), None).await,
        StatusCode::OK
    );
}

/// The allow-list's second arm. Announcements are the other half of the
/// public-root writers, and a single compound query now answers for both —
/// so exercise the announcement branch, not just the event one.
#[tokio::test]
async fn announcement_image_follows_its_public_flag() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (_, session) = make_session(&state, false).await;
    let public = announcement_with_image(&state, true).await;
    let members_only = announcement_with_image(&state, false).await;
    let app = coterie::web::create_web_routes(state.clone());

    assert_eq!(
        status_of(&app, &format!("/uploads/{}", public), None).await,
        StatusCode::OK
    );
    let uri = format!("/uploads/{}", members_only);
    assert_eq!(status_of(&app, &uri, None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(status_of(&app, &uri, Some(&session)).await, StatusCode::OK);
}

#[tokio::test]
async fn public_route_makes_no_attachment_decision() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (owner, _) = make_session(&state, false).await;
    let (_, filename) = event_with_image(&state, &pool, EventVisibility::Public).await;

    // 5.6 — point a submission row at the SAME file the public event
    // claims. Any surviving `submissions` lookup in `serve_upload` would
    // turn this into a refusal; the route must not consult that table at
    // all, so the public event's image keeps being served.
    let now = Utc::now();
    state
        .service_context
        .submission_repo
        .create(Submission {
            id: Uuid::new_v4(),
            submitter_member_id: owner,
            title: "Claims the same file".to_string(),
            abstract_text: "Body".to_string(),
            visibility_requested: SubmissionVisibility::Members,
            attachment_path: Some(format!("uploads/{}", filename)),
            preferred_start: None,
            timezone: "UTC".to_string(),
            duration_minutes: None,
            status: SubmissionStatus::Submitted,
            reviewer_note: None,
            decided_by: None,
            event_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("insert submission");

    let app = coterie::web::create_web_routes(state.clone());
    assert_eq!(
        status_of(&app, &format!("/uploads/{}", filename), None).await,
        StatusCode::OK,
        "serve_upload must not query submissions"
    );
}

// --- 5.8 / 5.9 The image allow-list fails closed -------------------------

#[tokio::test]
async fn deleting_a_members_only_event_leaves_its_image_unreachable() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (_, session) = make_session(&state, false).await;
    let (event, filename) = event_with_image(&state, &pool, EventVisibility::MembersOnly).await;
    let app = coterie::web::create_web_routes(state.clone());
    let uri = format!("/uploads/{}", filename);

    // Before: members-only → session required, member gets it.
    assert_eq!(status_of(&app, &uri, None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(status_of(&app, &uri, Some(&session)).await, StatusCode::OK);

    // 5.8 — after the row is gone, the old deny-list published the file.
    // The allow-list has nothing to affirm, so it refuses.
    SqliteEventRepository::new(pool.clone())
        .delete(event.id)
        .await
        .expect("delete event");
    assert_eq!(
        status_of(&app, &uri, None).await,
        StatusCode::UNAUTHORIZED,
        "an image whose row was deleted must not become anonymously readable"
    );
}

#[tokio::test]
async fn flipping_visibility_changes_anonymous_reach_without_moving_files() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (mut event, filename) = event_with_image(&state, &pool, EventVisibility::Public).await;
    let repo = SqliteEventRepository::new(pool.clone());
    let app = coterie::web::create_web_routes(state.clone());
    let uri = format!("/uploads/{}", filename);
    let on_disk = std::path::PathBuf::from(state.settings.server.uploads_path()).join(&filename);

    assert_eq!(status_of(&app, &uri, None).await, StatusCode::OK);

    // 5.9 — Public → MembersOnly → Public, same file, no relocation.
    event.visibility = EventVisibility::MembersOnly;
    event = repo.update(event.id, event.clone()).await.unwrap();
    assert_eq!(
        status_of(&app, &uri, None).await,
        StatusCode::UNAUTHORIZED,
        "flipping to members-only must take effect immediately"
    );
    assert!(on_disk.exists(), "the file must not have been moved");

    event.visibility = EventVisibility::Public;
    repo.update(event.id, event.clone()).await.unwrap();
    assert_eq!(
        status_of(&app, &uri, None).await,
        StatusCode::OK,
        "flipping back to public must restore anonymous access"
    );
    assert!(on_disk.exists(), "the file must not have been moved");
}

// --- 5.7 Migration -------------------------------------------------------

#[tokio::test]
async fn migration_moves_attachments_once_and_is_idempotent() {
    let pool = fresh_pool().await;
    let state = build_app_state(pool.clone()).await;
    let (owner, _) = make_session(&state, false).await;

    // A pre-a49 world: the attachment sits in the public root and the row
    // names it with the public prefix.
    let root = std::env::temp_dir().join(format!("coterie-a49-{}", Uuid::new_v4()));
    let public_dir = root.join("uploads");
    let private_dir = root.join("private-uploads");
    std::fs::create_dir_all(&public_dir).unwrap();
    let name = format!("{}.pdf", Uuid::new_v4());
    std::fs::write(public_dir.join(&name), b"%PDF-1.4 legacy").unwrap();
    // An unreferenced file the migration must leave alone.
    std::fs::write(public_dir.join("orphan.pdf"), b"%PDF-1.4 orphan").unwrap();

    let now = Utc::now();
    let id = state
        .service_context
        .submission_repo
        .create(Submission {
            id: Uuid::new_v4(),
            submitter_member_id: owner,
            title: "Legacy".to_string(),
            abstract_text: "Body".to_string(),
            visibility_requested: SubmissionVisibility::Members,
            attachment_path: Some(format!("uploads/{}", name)),
            preferred_start: None,
            timezone: "UTC".to_string(),
            duration_minutes: None,
            status: SubmissionStatus::Submitted,
            reviewer_note: None,
            decided_by: None,
            event_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap()
        .id;

    let dirs = (
        public_dir.to_str().unwrap().to_string(),
        private_dir.to_str().unwrap().to_string(),
    );
    migrate_attachments_to_private_root(&pool, &dirs.0, &dirs.1)
        .await
        .expect("first migration run");

    assert!(
        private_dir.join(&name).exists(),
        "file moved to the private root"
    );
    assert!(
        !public_dir.join(&name).exists(),
        "file left the public root"
    );
    assert!(
        public_dir.join("orphan.pdf").exists(),
        "unreferenced files stay put"
    );
    let path_after = |pool: SqlitePool| async move {
        sqlx::query_scalar::<_, String>("SELECT attachment_path FROM submissions WHERE id = ?")
            .bind(id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    let rewritten = path_after(pool.clone()).await;
    assert_eq!(rewritten, format!("private-uploads/{}", name));

    // 5.7 — a second run finds nothing to do and changes nothing.
    migrate_attachments_to_private_root(&pool, &dirs.0, &dirs.1)
        .await
        .expect("second migration run");
    assert_eq!(path_after(pool.clone()).await, rewritten);
    assert!(private_dir.join(&name).exists());
    assert_eq!(std::fs::read_dir(&private_dir).unwrap().count(), 1);
    assert_eq!(std::fs::read_dir(&public_dir).unwrap().count(), 1);

    std::fs::remove_dir_all(&root).ok();
}
