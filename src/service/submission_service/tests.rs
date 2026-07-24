use super::*;
use crate::{
    domain::CreateMemberRequest,
    integrations::IntegrationManager,
    repository::{
        EventRepository, EventSeriesRepository, MemberRepository, SqliteEventRepository,
        SqliteEventSeriesRepository, SqliteMemberRepository, SqliteSubmissionRepository,
        SubmissionRepository,
    },
    service::recurring_event_service::RecurringEventService,
};
use sqlx::{Executor, SqlitePool};

async fn fresh_pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _| {
            Box::pin(async move {
                conn.execute("PRAGMA foreign_keys = ON").await?;
                Ok(())
            })
        })
        .connect("sqlite::memory:")
        .await
        .expect(":memory:");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrate");
    pool
}

fn make_service(pool: SqlitePool) -> SubmissionService {
    let submission_repo: Arc<dyn SubmissionRepository> =
        Arc::new(SqliteSubmissionRepository::new(pool.clone()));
    let event_repo: Arc<dyn EventRepository> = Arc::new(SqliteEventRepository::new(pool.clone()));
    let event_series_repo: Arc<dyn EventSeriesRepository> =
        Arc::new(SqliteEventSeriesRepository::new(pool.clone()));
    let audit = Arc::new(AuditService::new(pool.clone()));
    let integrations = Arc::new(IntegrationManager::new());
    let recurring = Arc::new(RecurringEventService::new(
        event_repo.clone(),
        event_series_repo.clone(),
        pool.clone(),
    ));
    let event_admin = Arc::new(EventAdminService::new(
        event_repo,
        event_series_repo,
        recurring,
        audit.clone(),
        integrations,
    ));
    SubmissionService::new(submission_repo, event_admin, audit)
}

async fn make_member(pool: &SqlitePool) -> Uuid {
    let repo = SqliteMemberRepository::new(pool.clone());
    repo.create(CreateMemberRequest {
        email: format!("m-{}@example.com", Uuid::new_v4()),
        username: format!("u_{}", Uuid::new_v4().simple()),
        full_name: "Member".to_string(),
        password: "p4ssword_long_enough".to_string(),
        membership_type_id: None,
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}

fn create_input() -> CreateSubmissionInput {
    CreateSubmissionInput {
        title: "My Talk".to_string(),
        abstract_text: "A talk about things".to_string(),
        visibility_requested: SubmissionVisibility::Public,
        attachment_path: None,
        preferred_start: None,
        timezone: "UTC".to_string(),
        duration_minutes: None,
    }
}

#[tokio::test]
async fn create_persists_submitted_with_session_submitter() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let member = make_member(&pool).await;

    let created = svc.create(member, create_input()).await.unwrap();
    assert_eq!(created.status, SubmissionStatus::Submitted);
    // Submitter comes from the passed-in principal, never the body.
    assert_eq!(created.submitter_member_id, member);

    let fetched = svc.get_authorized(member, false, created.id).await.unwrap();
    assert_eq!(fetched.title, "My Talk");
}

#[tokio::test]
async fn oversized_title_and_abstract_are_rejected() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let member = make_member(&pool).await;

    let mut input = create_input();
    input.title = "x".repeat(201);
    assert!(matches!(
        svc.create(member, input).await,
        Err(AppError::Validation(_))
    ));

    let mut input = create_input();
    input.abstract_text = "y".repeat(5001);
    assert!(matches!(
        svc.create(member, input).await,
        Err(AppError::Validation(_))
    ));

    // Nothing persisted.
    let repo = SqliteSubmissionRepository::new(pool.clone());
    assert_eq!(repo.count_open_for_member(member).await.unwrap(), 0);
}

#[tokio::test]
async fn cross_member_read_is_denied_without_disclosure() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let other = make_member(&pool).await;

    let created = svc.create(owner, create_input()).await.unwrap();

    // Member B reading A's submission → NotFound (undisclosed).
    let err = svc
        .get_authorized(other, false, created.id)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));

    // Owner succeeds; an admin (is_admin=true) is exempt.
    assert!(svc.get_authorized(owner, false, created.id).await.is_ok());
    assert!(svc.get_authorized(other, true, created.id).await.is_ok());
}

#[tokio::test]
async fn non_owner_cannot_edit_or_withdraw() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let other = make_member(&pool).await;
    let created = svc.create(owner, create_input()).await.unwrap();

    let update = UpdateSubmissionInput {
        title: "Hijacked".to_string(),
        abstract_text: "x".to_string(),
        visibility_requested: SubmissionVisibility::Members,
        preferred_start: None,
        timezone: "UTC".to_string(),
        duration_minutes: None,
        new_attachment_path: None,
    };
    assert!(matches!(
        svc.update_owned(other, created.id, update).await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        svc.withdraw_owned(other, created.id).await,
        Err(AppError::NotFound(_))
    ));

    // Owner withdraw succeeds → terminal.
    let withdrawn = svc.withdraw_owned(owner, created.id).await.unwrap();
    assert_eq!(withdrawn.status, SubmissionStatus::Withdrawn);
}

#[tokio::test]
async fn edit_after_review_started_is_refused() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let admin = make_member(&pool).await;
    let created = svc.create(owner, create_input()).await.unwrap();

    svc.start_review(admin, created.id).await.unwrap();

    let update = UpdateSubmissionInput {
        title: "Late edit".to_string(),
        abstract_text: "x".to_string(),
        visibility_requested: SubmissionVisibility::Members,
        preferred_start: None,
        timezone: "UTC".to_string(),
        duration_minutes: None,
        new_attachment_path: None,
    };
    assert!(matches!(
        svc.update_owned(owner, created.id, update).await,
        Err(AppError::Validation(_))
    ));
}

#[tokio::test]
async fn open_submission_cap_is_enforced() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let member = make_member(&pool).await;

    for _ in 0..MAX_OPEN_SUBMISSIONS {
        svc.create(member, create_input()).await.unwrap();
    }
    // The (MAX+1)th is refused with a clear error.
    let err = svc.create(member, create_input()).await.unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));

    // Withdrawing one frees a slot.
    let mine = SqliteSubmissionRepository::new(pool.clone())
        .list_for_member(member)
        .await
        .unwrap();
    svc.withdraw_owned(member, mine[0].id).await.unwrap();
    assert!(svc.create(member, create_input()).await.is_ok());
}

#[tokio::test]
async fn accept_with_schedule_creates_event_with_matching_visibility() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let admin = make_member(&pool).await;

    // Public-requested submission.
    let created = svc.create(owner, create_input()).await.unwrap();

    let schedule = PromoteSchedule {
        start: DateTime::from_naive_utc_and_offset(
            chrono::NaiveDate::from_ymd_opt(2026, 9, 1)
                .unwrap()
                .and_hms_opt(18, 0, 0)
                .unwrap(),
            Utc,
        ),
        timezone: "UTC".to_string(),
        duration_minutes: Some(60),
    };
    let accepted = svc
        .accept(
            admin,
            created.id,
            Some("Great talk".to_string()),
            Some(schedule),
        )
        .await
        .unwrap();

    assert_eq!(accepted.status, SubmissionStatus::Scheduled);
    let event_id = accepted.event_id.expect("event created on promotion");
    assert_eq!(accepted.decided_by, Some(admin));

    // The created event mirrors the accepted (public) visibility.
    let event_repo = SqliteEventRepository::new(pool.clone());
    let event = event_repo.find_by_id(event_id).await.unwrap().unwrap();
    assert_eq!(event.visibility, EventVisibility::Public);
    assert_eq!(event.title, "My Talk");
}

#[tokio::test]
async fn accept_without_schedule_does_not_create_event() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let admin = make_member(&pool).await;
    let created = svc.create(owner, create_input()).await.unwrap();

    let accepted = svc.accept(admin, created.id, None, None).await.unwrap();
    assert_eq!(accepted.status, SubmissionStatus::Accepted);
    assert!(accepted.event_id.is_none());
}

#[tokio::test]
async fn cannot_decide_a_terminal_submission() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let admin = make_member(&pool).await;
    let created = svc.create(owner, create_input()).await.unwrap();

    svc.withdraw_owned(owner, created.id).await.unwrap();
    // Withdrawn is terminal — accept/decline refused.
    assert!(matches!(
        svc.accept(admin, created.id, None, None).await,
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        svc.decline(admin, created.id, None).await,
        Err(AppError::BadRequest(_))
    ));
}

/// A throwaway uploads dir under the system temp holding one file that
/// stands in for a submission attachment. Returns (uploads_dir, rel_path,
/// on_disk_path); `rel_path` is the `uploads/<name>` form persisted on the
/// submission.
fn temp_uploads_with_file() -> (std::path::PathBuf, String, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("coterie-sub-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let name = format!("{}.pdf", Uuid::new_v4());
    let on_disk = dir.join(&name);
    std::fs::write(&on_disk, b"%PDF-1.4 test").unwrap();
    (dir, format!("uploads/{}", name), on_disk)
}

#[tokio::test]
async fn owner_deletes_withdrawn_removes_row_and_attachment() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let repo = SqliteSubmissionRepository::new(pool.clone());

    let (uploads_dir, rel, on_disk) = temp_uploads_with_file();
    let mut input = create_input();
    input.attachment_path = Some(rel);
    let created = svc.create(owner, input).await.unwrap();
    svc.withdraw_owned(owner, created.id).await.unwrap();
    assert!(on_disk.exists());

    svc.delete(owner, created.id, uploads_dir.to_str().unwrap())
        .await
        .unwrap();

    // Row gone AND the attachment file removed.
    assert!(repo.find_by_id(created.id).await.unwrap().is_none());
    assert!(!on_disk.exists());

    // Don't leave the (now empty) temp dir behind in /tmp.
    std::fs::remove_dir_all(&uploads_dir).ok();
}

#[tokio::test]
async fn owner_deletes_declined_removes_row() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let admin = make_member(&pool).await;
    let repo = SqliteSubmissionRepository::new(pool.clone());

    let created = svc.create(owner, create_input()).await.unwrap();
    svc.decline(admin, created.id, Some("Not this time".to_string()))
        .await
        .unwrap();

    svc.delete(owner, created.id, "/nonexistent")
        .await
        .unwrap();
    assert!(repo.find_by_id(created.id).await.unwrap().is_none());
}

#[tokio::test]
async fn delete_of_active_state_is_refused_and_unchanged() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let admin = make_member(&pool).await;
    let repo = SqliteSubmissionRepository::new(pool.clone());

    // submitted
    let submitted = svc.create(owner, create_input()).await.unwrap();
    // under_review
    let under_review = svc.create(owner, create_input()).await.unwrap();
    svc.start_review(admin, under_review.id).await.unwrap();
    // accepted (no schedule)
    let accepted = svc.create(owner, create_input()).await.unwrap();
    svc.accept(admin, accepted.id, None, None).await.unwrap();
    // scheduled (accepted with a schedule)
    let scheduled = svc.create(owner, create_input()).await.unwrap();
    let schedule = PromoteSchedule {
        start: DateTime::from_naive_utc_and_offset(
            chrono::NaiveDate::from_ymd_opt(2026, 9, 1)
                .unwrap()
                .and_hms_opt(18, 0, 0)
                .unwrap(),
            Utc,
        ),
        timezone: "UTC".to_string(),
        duration_minutes: Some(60),
    };
    svc.accept(admin, scheduled.id, None, Some(schedule))
        .await
        .unwrap();

    for id in [submitted.id, under_review.id, accepted.id, scheduled.id] {
        assert!(matches!(
            svc.delete(owner, id, "/nonexistent").await,
            Err(AppError::Validation(_))
        ));
        // Untouched.
        assert!(repo.find_by_id(id).await.unwrap().is_some());
    }
}

/// The repo `delete` is guarded on terminal status in SQL, so a row that
/// stopped being terminal between the service's load and delete (a
/// concurrent re-open) is NOT removed — the guard, not just the app-level
/// check, refuses. Exercised directly since the race is hard to stage
/// through the service.
#[tokio::test]
async fn repo_delete_guard_refuses_non_terminal_row() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let repo = SqliteSubmissionRepository::new(pool.clone());

    // `submitted` (active) → guard matches nothing, returns false, row stays.
    let submitted = svc.create(owner, create_input()).await.unwrap();
    assert!(!repo.delete(submitted.id).await.unwrap());
    assert!(repo.find_by_id(submitted.id).await.unwrap().is_some());

    // `withdrawn` (terminal) → guard matches, returns true, row gone.
    svc.withdraw_owned(owner, submitted.id).await.unwrap();
    assert!(repo.delete(submitted.id).await.unwrap());
    assert!(repo.find_by_id(submitted.id).await.unwrap().is_none());
}

#[tokio::test]
async fn owner_reopens_withdrawn_and_it_becomes_editable() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let admin = make_member(&pool).await;

    let created = svc.create(owner, create_input()).await.unwrap();
    svc.withdraw_owned(owner, created.id).await.unwrap();

    let reopened = svc.reopen(owner, created.id).await.unwrap();
    assert_eq!(reopened.status, SubmissionStatus::Submitted);

    // Editable again: an owner edit now succeeds.
    let update = UpdateSubmissionInput {
        title: "Revised".to_string(),
        abstract_text: "Now improved".to_string(),
        visibility_requested: SubmissionVisibility::Members,
        preferred_start: None,
        timezone: "UTC".to_string(),
        duration_minutes: None,
        new_attachment_path: None,
    };
    let edited = svc.update_owned(owner, created.id, update).await.unwrap();
    assert_eq!(edited.title, "Revised");

    // Re-open of a non-withdrawn (now submitted) → refused.
    assert!(matches!(
        svc.reopen(owner, created.id).await,
        Err(AppError::Validation(_))
    ));

    // Re-open of a declined → refused (a decision is preserved, not resurrected).
    let declined = svc.create(owner, create_input()).await.unwrap();
    svc.decline(admin, declined.id, None).await.unwrap();
    assert!(matches!(
        svc.reopen(owner, declined.id).await,
        Err(AppError::Validation(_))
    ));
}

#[tokio::test]
async fn non_owner_cannot_delete_or_reopen() {
    let pool = fresh_pool().await;
    let svc = make_service(pool.clone());
    let owner = make_member(&pool).await;
    let other = make_member(&pool).await;
    let repo = SqliteSubmissionRepository::new(pool.clone());

    let created = svc.create(owner, create_input()).await.unwrap();
    svc.withdraw_owned(owner, created.id).await.unwrap();

    // A non-owner is denied WITHOUT disclosure (404), not a state error.
    assert!(matches!(
        svc.delete(other, created.id, "/nonexistent").await,
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        svc.reopen(other, created.id).await,
        Err(AppError::NotFound(_))
    ));

    // Untouched — still present and still withdrawn.
    let still = repo.find_by_id(created.id).await.unwrap().unwrap();
    assert_eq!(still.status, SubmissionStatus::Withdrawn);
}
