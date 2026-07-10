//! Integration tests for `AnnouncementAdminService::publish_scheduled`
//! and the underlying repository method `mark_published_now`. Hits a
//! real in-memory SQLite + migrations and asserts on the full
//! side-effect chain: row transition (Draft → Published), audit row
//! (with NULL actor — system-initiated), and integration dispatch
//! (`AnnouncementPublished`) via a fake Integration.
//!
//! Run: cargo test --features test-utils --test scheduled_announcement_publish_test

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use coterie::{
    domain::{Announcement, AnnouncementType, CreateMemberRequest},
    error::Result as CoterieResult,
    integrations::{Integration, IntegrationEvent, IntegrationManager},
    repository::{
        AnnouncementRepository, MemberRepository, SqliteAnnouncementRepository,
        SqliteMemberRepository,
    },
    service::{announcement_admin_service::AnnouncementAdminService, audit_service::AuditService},
};
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use uuid::Uuid;

mod common;
use common::fresh_pool;

// ---------------------------------------------------------------------
// Fake Integration — records every event handled.
// ---------------------------------------------------------------------

struct FakeIntegration {
    name: String,
    events: Mutex<Vec<IntegrationEvent>>,
}

impl FakeIntegration {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            name: "fake".to_string(),
            events: Mutex::new(Vec::new()),
        })
    }

    async fn announcement_published_count(&self) -> usize {
        self.events
            .lock()
            .await
            .iter()
            .filter(|e| matches!(e, IntegrationEvent::AnnouncementPublished(_)))
            .count()
    }
}

#[async_trait]
impl Integration for FakeIntegration {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_enabled(&self) -> bool {
        true
    }
    async fn health_check(&self) -> CoterieResult<()> {
        Ok(())
    }
    async fn handle_event(&self, event: &IntegrationEvent) -> CoterieResult<()> {
        self.events.lock().await.push(event.clone());
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct H {
    pool: SqlitePool,
    repo: Arc<dyn AnnouncementRepository>,
    service: AnnouncementAdminService,
    fake_integration: Arc<FakeIntegration>,
    actor: Uuid,
}

async fn build() -> H {
    let pool = fresh_pool().await;
    let repo: Arc<dyn AnnouncementRepository> =
        Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
    let audit = Arc::new(AuditService::new(pool.clone()));
    let manager = Arc::new(IntegrationManager::new());
    let fake = FakeIntegration::new();
    manager.register(fake.clone() as Arc<dyn Integration>).await;

    let service = AnnouncementAdminService::new(repo.clone(), audit, manager);

    // Seed an actor (creator) — needed to satisfy the foreign key.
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let actor = member_repo
        .create(CreateMemberRequest {
            email: format!("creator-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Creator".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("create member")
        .id;

    H {
        pool,
        repo,
        service,
        fake_integration: fake,
        actor,
    }
}

async fn seed_announcement(
    h: &H,
    published_at: Option<DateTime<Utc>>,
    scheduled_publish_at: Option<DateTime<Utc>>,
    scheduled_publish_timezone: &str,
) -> Announcement {
    let now = Utc::now();
    let row = Announcement {
        id: Uuid::new_v4(),
        title: "Scheduled".to_string(),
        content: "Body".to_string(),
        announcement_type: AnnouncementType::General,
        announcement_type_id: None,
        is_public: false,
        featured: false,
        image_url: None,
        published_at,
        scheduled_publish_at,
        scheduled_publish_timezone: scheduled_publish_timezone.to_string(),
        created_by: h.actor,
        created_at: now,
        updated_at: now,
    };
    h.repo.create(row).await.expect("seed announcement")
}

async fn audit_count(pool: &SqlitePool, action: &str, entity_id: &str) -> i64 {
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = ? AND entity_id = ?")
            .bind(action)
            .bind(entity_id)
            .fetch_one(pool)
            .await
            .expect("audit count");
    count.0
}

async fn audit_actor_is_null(pool: &SqlitePool, action: &str, entity_id: &str) -> bool {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT actor_id FROM audit_logs WHERE action = ? AND entity_id = ?")
            .bind(action)
            .bind(entity_id)
            .fetch_optional(pool)
            .await
            .expect("actor_id query");
    matches!(row, Some((None,)))
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn past_due_draft_fires_publishes_audits_and_dispatches() {
    let h = build().await;

    let scheduled = Utc::now() - Duration::minutes(5);
    let row = seed_announcement(&h, None, Some(scheduled), "UTC").await;

    let count = h
        .service
        .publish_scheduled()
        .await
        .expect("publish_scheduled");

    assert_eq!(count, 1, "exactly one row should have been published");

    // Row is now Published.
    let fetched = h
        .repo
        .find_by_id(row.id)
        .await
        .unwrap()
        .expect("row exists");
    assert!(
        fetched.published_at.is_some(),
        "row should be Published after publish_scheduled"
    );
    assert!(
        fetched.scheduled_publish_at.is_none(),
        "scheduled_publish_at should be cleared on transition"
    );

    // Audit row exists with NULL actor.
    assert_eq!(
        audit_count(&h.pool, "auto_publish_announcement", &row.id.to_string()).await,
        1,
    );
    assert!(
        audit_actor_is_null(&h.pool, "auto_publish_announcement", &row.id.to_string()).await,
        "system-initiated audit row should have actor_id IS NULL",
    );

    // Integration dispatched once.
    assert_eq!(h.fake_integration.announcement_published_count().await, 1);
}

#[tokio::test]
async fn future_draft_is_not_touched() {
    let h = build().await;

    let scheduled = Utc::now() + Duration::hours(2);
    let row = seed_announcement(&h, None, Some(scheduled), "UTC").await;

    let count = h
        .service
        .publish_scheduled()
        .await
        .expect("publish_scheduled");

    assert_eq!(count, 0, "future-scheduled row should not be published");

    let fetched = h
        .repo
        .find_by_id(row.id)
        .await
        .unwrap()
        .expect("row exists");
    assert!(fetched.published_at.is_none(), "row should still be Draft");
    assert!(
        fetched.scheduled_publish_at.is_some(),
        "scheduled_publish_at should be preserved",
    );

    assert_eq!(
        audit_count(&h.pool, "auto_publish_announcement", &row.id.to_string()).await,
        0,
    );
    assert_eq!(h.fake_integration.announcement_published_count().await, 0);
}

#[tokio::test]
async fn already_published_row_is_not_double_dispatched() {
    let h = build().await;

    // Edge case: an already-Published row that somehow still has a
    // past-due scheduled_publish_at. The runner should not touch it.
    let already_published = Some(Utc::now() - Duration::hours(1));
    let scheduled = Some(Utc::now() - Duration::minutes(5));
    let row = seed_announcement(&h, already_published, scheduled, "UTC").await;

    let count = h
        .service
        .publish_scheduled()
        .await
        .expect("publish_scheduled");

    assert_eq!(count, 0, "Published rows must not be re-fired");

    let fetched = h
        .repo
        .find_by_id(row.id)
        .await
        .unwrap()
        .expect("row exists");
    assert!(
        fetched.published_at.is_some(),
        "row should remain Published",
    );

    assert_eq!(
        audit_count(&h.pool, "auto_publish_announcement", &row.id.to_string()).await,
        0,
    );
    assert_eq!(h.fake_integration.announcement_published_count().await, 0);
}

#[tokio::test]
async fn concurrent_mark_published_now_only_one_wins() {
    // Two concurrent calls to mark_published_now on the same Draft row.
    // Exactly one SHALL return true; the other SHALL return false. The
    // atomic conditional UPDATE is what guarantees this.
    let h = build().await;
    let scheduled = Utc::now() - Duration::minutes(5);
    let row = seed_announcement(&h, None, Some(scheduled), "UTC").await;

    let repo_a = h.repo.clone();
    let repo_b = h.repo.clone();
    let id = row.id;

    let task_a = tokio::spawn(async move { repo_a.mark_published_now(id).await });
    let task_b = tokio::spawn(async move { repo_b.mark_published_now(id).await });

    let result_a = task_a.await.expect("task_a join").expect("repo a");
    let result_b = task_b.await.expect("task_b join").expect("repo b");

    let winners = [result_a, result_b].iter().filter(|x| **x).count();
    assert_eq!(
        winners, 1,
        "exactly one of the two concurrent calls must win"
    );
}

// Task 4.1 — the core timezone fix. An America/New_York (EDT, UTC-4 in
// summer) org schedules a Draft for a wall-clock one hour in the PAST.
// Its true instant is +4/+5h from the wall-clock, i.e. still ~3-4h in the
// FUTURE. The old bug compared the raw wall-clock (already past) to `now`
// and published up to four hours early; the fix compares the DERIVED
// instant and holds the row.
#[tokio::test]
async fn non_utc_org_draft_does_not_publish_early() {
    let h = build().await;

    let wall_naive = (Utc::now() - Duration::hours(1)).naive_utc();
    let wall = DateTime::from_naive_utc_and_offset(wall_naive, Utc);
    let row = seed_announcement(&h, None, Some(wall), "America/New_York").await;

    // Sanity: the derived instant IS still in the future — otherwise the
    // test would prove nothing about early publishing.
    let fetched = h.repo.find_by_id(row.id).await.unwrap().unwrap();
    assert!(
        fetched.scheduled_publish_utc().unwrap() > Utc::now(),
        "New York wall-clock (now-1h) should derive to a future instant",
    );

    let count = h
        .service
        .publish_scheduled()
        .await
        .expect("publish_scheduled");
    assert_eq!(
        count, 0,
        "must not publish before the true instant (no offset-early fire)"
    );

    let after = h.repo.find_by_id(row.id).await.unwrap().unwrap();
    assert!(after.published_at.is_none(), "row should still be Draft");
    assert_eq!(h.fake_integration.announcement_published_count().await, 0);
}

// A New York Draft whose DERIVED instant is already past DOES fire — the
// runner publishes on the true instant, not offset-hours late either.
#[tokio::test]
async fn non_utc_org_draft_fires_once_true_instant_passes() {
    let h = build().await;

    // Wall-clock 8h in the past → derived ~4-5h in the past for NY → due.
    let wall_naive = (Utc::now() - Duration::hours(8)).naive_utc();
    let wall = DateTime::from_naive_utc_and_offset(wall_naive, Utc);
    let row = seed_announcement(&h, None, Some(wall), "America/New_York").await;

    let count = h
        .service
        .publish_scheduled()
        .await
        .expect("publish_scheduled");
    assert_eq!(count, 1, "past-true-instant NY Draft should publish");

    let after = h.repo.find_by_id(row.id).await.unwrap().unwrap();
    assert!(after.published_at.is_some(), "row should be Published");
    assert_eq!(h.fake_integration.announcement_published_count().await, 1);
}

// Task 4.2 — the zone column is a pure annotation. Two Drafts with the
// SAME wall-clock but different frozen zones store and render the SAME
// wall-clock value; only the DERIVED instant differs. (This mirrors what
// the migration does: add + backfill the zone, shift no stored value.)
#[tokio::test]
async fn zone_annotation_does_not_shift_stored_wallclock() {
    let h = build().await;
    let wall_naive = (Utc::now() + Duration::days(3)).naive_utc();
    let wall = DateTime::from_naive_utc_and_offset(wall_naive, Utc);

    let utc_row = seed_announcement(&h, None, Some(wall), "UTC").await;
    let ny_row = seed_announcement(&h, None, Some(wall), "America/New_York").await;

    let utc_fetched = h.repo.find_by_id(utc_row.id).await.unwrap().unwrap();
    let ny_fetched = h.repo.find_by_id(ny_row.id).await.unwrap().unwrap();

    // Stored (and hence admin-rendered) wall-clock is identical.
    assert_eq!(
        utc_fetched.scheduled_publish_at, ny_fetched.scheduled_publish_at,
        "the zone must not shift the stored wall-clock",
    );
    // But the derived instant reflects the NY offset (~4-5h earlier UTC).
    assert_ne!(
        utc_fetched.scheduled_publish_utc(),
        ny_fetched.scheduled_publish_utc(),
        "derived instant should reflect the frozen zone offset",
    );
}
