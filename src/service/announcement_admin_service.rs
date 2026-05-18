//! Service that owns the full side-effect chain for admin-driven
//! announcement mutations: repo update → audit log → integration
//! dispatch. Handlers parse the wire shape and render the response;
//! the service owns everything between.
//!
//! Mirrors `EventAdminService`'s shape — a per-domain service that
//! co-locates validation, persistence, and the post-work chain so a
//! contributor adding a new admin action can't accidentally forget
//! one piece (audit, integration event). See the
//! `announcement-admin-service` capability spec for the contract.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    domain::{Announcement, AnnouncementType},
    error::{AppError, Result},
    integrations::{IntegrationEvent, IntegrationManager},
    repository::AnnouncementRepository,
    service::audit_service::AuditService,
};

/// Typed input for creating an announcement. The handler parses the
/// multipart form into one of these and hands it off. When
/// `publish_now` is true the service stamps `published_at` and
/// dispatches `IntegrationEvent::AnnouncementPublished`; when false
/// the row is persisted as a Draft.
pub struct CreateAnnouncementInput {
    pub title: String,
    pub content: String,
    pub announcement_type: AnnouncementType,
    pub announcement_type_id: Option<Uuid>,
    pub is_public: bool,
    pub featured: bool,
    pub image_url: Option<String>,
    pub publish_now: bool,
    /// Optional future-publish time. Ignored when `publish_now` is
    /// true (publish-now wins). A Draft row with this set is what the
    /// background runner picks up at-or-after the scheduled time.
    pub scheduled_publish_at: Option<DateTime<Utc>>,
}

/// Typed input for updating an announcement. Carries the editable
/// subset of `Announcement` fields; immutable identity fields (id,
/// created_by, created_at, published_at) are not part of this — the
/// service preserves them from the existing row.
pub struct UpdateAnnouncementInput {
    pub title: String,
    pub content: String,
    pub announcement_type: AnnouncementType,
    pub announcement_type_id: Option<Uuid>,
    pub is_public: bool,
    pub featured: bool,
    pub image_url: Option<String>,
    /// Optional future-publish time. Persisted as-is on the row;
    /// empty/None clears any prior schedule.
    pub scheduled_publish_at: Option<DateTime<Utc>>,
}

pub struct AnnouncementAdminService {
    announcement_repo: Arc<dyn AnnouncementRepository>,
    audit_service: Arc<AuditService>,
    integration_manager: Arc<IntegrationManager>,
}

impl AnnouncementAdminService {
    pub fn new(
        announcement_repo: Arc<dyn AnnouncementRepository>,
        audit_service: Arc<AuditService>,
        integration_manager: Arc<IntegrationManager>,
    ) -> Self {
        Self {
            announcement_repo,
            audit_service,
            integration_manager,
        }
    }

    /// Create an announcement. If `input.publish_now` is true, stamps
    /// `published_at = now` and dispatches
    /// `IntegrationEvent::AnnouncementPublished`. Otherwise persists
    /// as a Draft and only audits.
    pub async fn create(
        &self,
        actor_id: Uuid,
        input: CreateAnnouncementInput,
    ) -> Result<Announcement> {
        let now = Utc::now();
        let published_at = if input.publish_now { Some(now) } else { None };
        // publish-now wins per spec: drop any scheduled_publish_at the
        // form may have set if the admin also ticked "publish now".
        let scheduled_publish_at = if input.publish_now {
            None
        } else {
            input.scheduled_publish_at
        };

        let announcement = Announcement {
            id: Uuid::new_v4(),
            title: input.title,
            content: input.content,
            announcement_type: input.announcement_type,
            announcement_type_id: input.announcement_type_id,
            is_public: input.is_public,
            featured: input.featured,
            image_url: input.image_url,
            published_at,
            scheduled_publish_at,
            created_by: actor_id,
            created_at: now,
            updated_at: now,
        };

        let created = self.announcement_repo.create(announcement).await?;

        self.audit_service.log(
            Some(actor_id),
            "create_announcement",
            "announcement",
            &created.id.to_string(),
            None,
            Some(&created.title),
            None,
        ).await;

        // If the admin chose "publish now", treat that the same as a
        // separate publish action — fire the integration event so
        // Discord posts to the announcements channel. Drafts (no
        // published_at) don't fire; they'll fire when the admin hits
        // the Publish button later.
        if created.published_at.is_some() {
            self.integration_manager
                .handle_event(IntegrationEvent::AnnouncementPublished(created.clone()))
                .await;
        }

        Ok(created)
    }

    /// Update an announcement. Preserves `published_at`, `created_by`,
    /// and `created_at` from the existing row. Audits `update_announcement`.
    /// No integration dispatch — updates are silent.
    pub async fn update(
        &self,
        actor_id: Uuid,
        announcement_id: Uuid,
        input: UpdateAnnouncementInput,
    ) -> Result<Announcement> {
        let existing = self.announcement_repo.find_by_id(announcement_id).await?
            .ok_or_else(|| AppError::NotFound("Announcement not found".to_string()))?;

        let updated = Announcement {
            id: announcement_id,
            title: input.title,
            content: input.content,
            announcement_type: input.announcement_type,
            announcement_type_id: input.announcement_type_id,
            is_public: input.is_public,
            featured: input.featured,
            image_url: input.image_url,
            published_at: existing.published_at,
            scheduled_publish_at: input.scheduled_publish_at,
            created_by: existing.created_by,
            created_at: existing.created_at,
            updated_at: Utc::now(),
        };

        let result = self.announcement_repo.update(announcement_id, updated).await?;

        self.audit_service.log(
            Some(actor_id),
            "update_announcement",
            "announcement",
            &announcement_id.to_string(),
            None,
            Some(&result.title),
            None,
        ).await;

        Ok(result)
    }

    /// Delete an announcement. Audits `delete_announcement`.
    pub async fn delete(&self, actor_id: Uuid, announcement_id: Uuid) -> Result<()> {
        self.announcement_repo.delete(announcement_id).await?;
        self.audit_service.log(
            Some(actor_id),
            "delete_announcement",
            "announcement",
            &announcement_id.to_string(),
            None,
            None,
            None,
        ).await;
        Ok(())
    }

    /// Publish a Draft announcement. Idempotent: re-publishing an
    /// already-published row updates `updated_at` and writes an audit
    /// row but does NOT re-dispatch the integration event.
    pub async fn publish(
        &self,
        actor_id: Uuid,
        announcement_id: Uuid,
    ) -> Result<Announcement> {
        let existing = self.announcement_repo.find_by_id(announcement_id).await?
            .ok_or_else(|| AppError::NotFound("Announcement not found".to_string()))?;

        let was_already_published = existing.published_at.is_some();
        let mut updated = existing;
        updated.published_at = Some(Utc::now());
        updated.updated_at = Utc::now();

        let saved = self.announcement_repo.update(announcement_id, updated).await?;

        self.audit_service.log(
            Some(actor_id),
            "publish_announcement",
            "announcement",
            &announcement_id.to_string(),
            None,
            Some(&saved.title),
            None,
        ).await;

        // Only fire the integration event on the transition from
        // unpublished → published, not on re-publishing an already-
        // public announcement (which can happen if an admin clicks
        // Publish twice for some reason).
        if !was_already_published {
            self.integration_manager
                .handle_event(IntegrationEvent::AnnouncementPublished(saved.clone()))
                .await;
        }

        Ok(saved)
    }

    /// Runner entry point. Finds Draft announcements whose
    /// `scheduled_publish_at <= now`, atomically flips each to
    /// Published (via the repo's conditional UPDATE), and on each
    /// successful claim writes an `auto_publish_announcement` audit
    /// row (actor_id = None — system action) and dispatches
    /// `IntegrationEvent::AnnouncementPublished`. Returns the number
    /// of rows published. Errors on individual rows are logged but do
    /// not stop the loop.
    pub async fn publish_scheduled(&self) -> Result<u32> {
        let now = Utc::now();
        let candidates = self.announcement_repo.list_due_for_publish(now).await?;
        let mut sent: u32 = 0;
        for candidate in candidates {
            match self.announcement_repo.mark_published_now(candidate.id).await {
                Ok(true) => {
                    // Re-fetch so the row carries the updated
                    // `published_at` and the cleared schedule. This
                    // costs one extra read per row but keeps the
                    // integration event payload accurate.
                    let published = match self.announcement_repo.find_by_id(candidate.id).await {
                        Ok(Some(a)) => a,
                        Ok(None) => continue,
                        Err(e) => {
                            tracing::error!("publish_scheduled: refetch failed for {}: {}", candidate.id, e);
                            continue;
                        }
                    };
                    self.audit_service.log(
                        None,
                        "auto_publish_announcement",
                        "announcement",
                        &published.id.to_string(),
                        None,
                        Some(&published.title),
                        None,
                    ).await;
                    self.integration_manager
                        .handle_event(IntegrationEvent::AnnouncementPublished(published))
                        .await;
                    sent += 1;
                }
                Ok(false) => {
                    // Lost the race or status changed under us; skip.
                }
                Err(e) => {
                    tracing::error!("publish_scheduled: mark_published_now failed for {}: {}", candidate.id, e);
                }
            }
        }
        Ok(sent)
    }

    /// Unpublish a Published announcement (back to Draft). Audits
    /// `unpublish_announcement`. No integration dispatch — unpublish
    /// is silent on the integration channel.
    pub async fn unpublish(
        &self,
        actor_id: Uuid,
        announcement_id: Uuid,
    ) -> Result<Announcement> {
        let existing = self.announcement_repo.find_by_id(announcement_id).await?
            .ok_or_else(|| AppError::NotFound("Announcement not found".to_string()))?;

        let mut updated = existing;
        updated.published_at = None;
        updated.updated_at = Utc::now();

        let saved = self.announcement_repo.update(announcement_id, updated).await?;

        self.audit_service.log(
            Some(actor_id),
            "unpublish_announcement",
            "announcement",
            &announcement_id.to_string(),
            None,
            Some(&saved.title),
            None,
        ).await;

        Ok(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::CreateMemberRequest,
        integrations::IntegrationManager,
        repository::{
            MemberRepository, SqliteAnnouncementRepository, SqliteMemberRepository,
        },
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
        sqlx::migrate!("./migrations").run(&pool).await.expect("migrate");
        pool
    }

    fn make_service(pool: SqlitePool) -> AnnouncementAdminService {
        let announcement_repo: Arc<dyn AnnouncementRepository> =
            Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
        let audit = Arc::new(AuditService::new(pool.clone()));
        let integrations = Arc::new(IntegrationManager::new());

        AnnouncementAdminService::new(announcement_repo, audit, integrations)
    }

    async fn make_actor(pool: &SqlitePool) -> Uuid {
        let repo = SqliteMemberRepository::new(pool.clone());
        let m = repo.create(CreateMemberRequest {
            email: format!("a-{}@example.com", Uuid::new_v4()),
            username: format!("u_{}", Uuid::new_v4().simple()),
            full_name: "Test Admin".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
        }).await.unwrap();
        m.id
    }

    async fn audit_count(pool: &SqlitePool, action: &str, entity_id: &str) -> i64 {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_logs WHERE action = ? AND entity_id = ?"
        )
        .bind(action)
        .bind(entity_id)
        .fetch_one(pool).await.unwrap();
        count.0
    }

    fn create_input(publish_now: bool) -> CreateAnnouncementInput {
        CreateAnnouncementInput {
            title: "Test Announcement".to_string(),
            content: "Body text".to_string(),
            announcement_type: AnnouncementType::General,
            announcement_type_id: None,
            is_public: false,
            featured: false,
            image_url: None,
            publish_now,
            scheduled_publish_at: None,
        }
    }

    #[tokio::test]
    async fn create_draft_does_not_dispatch_integration() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let announcement = svc.create(actor, create_input(false)).await.unwrap();

        // Draft: published_at None.
        assert!(announcement.published_at.is_none(), "draft should have no published_at");

        // Repo touched — row exists.
        let fetched = svc.announcement_repo.find_by_id(announcement.id).await.unwrap().unwrap();
        assert_eq!(fetched.title, "Test Announcement");
        assert!(fetched.published_at.is_none());

        // Audit row inserted.
        assert_eq!(audit_count(&pool, "create_announcement", &announcement.id.to_string()).await, 1);

        // No external observability of the integration_manager call
        // beyond reaching here without panicking — IntegrationManager
        // has no registered integrations in this test.
    }

    #[tokio::test]
    async fn create_publish_now_dispatches_integration() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let announcement = svc.create(actor, create_input(true)).await.unwrap();

        assert!(announcement.published_at.is_some(), "publish_now should stamp published_at");

        let fetched = svc.announcement_repo.find_by_id(announcement.id).await.unwrap().unwrap();
        assert!(fetched.published_at.is_some());

        assert_eq!(audit_count(&pool, "create_announcement", &announcement.id.to_string()).await, 1);
    }

    #[tokio::test]
    async fn update_writes_audit_and_preserves_published_at() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        // Create a Published announcement.
        let announcement = svc.create(actor, create_input(true)).await.unwrap();
        let original_published_at = announcement.published_at;

        let input = UpdateAnnouncementInput {
            title: "Renamed".to_string(),
            content: "New body".to_string(),
            announcement_type: AnnouncementType::News,
            announcement_type_id: None,
            is_public: true,
            featured: true,
            image_url: None,
            scheduled_publish_at: None,
        };

        let result = svc.update(actor, announcement.id, input).await.unwrap();

        assert_eq!(result.title, "Renamed");
        assert_eq!(result.content, "New body");
        assert_eq!(result.published_at, original_published_at, "update should preserve published_at");
        assert_eq!(audit_count(&pool, "update_announcement", &announcement.id.to_string()).await, 1);
    }

    #[tokio::test]
    async fn delete_writes_audit() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let announcement = svc.create(actor, create_input(false)).await.unwrap();

        svc.delete(actor, announcement.id).await.unwrap();
        assert!(svc.announcement_repo.find_by_id(announcement.id).await.unwrap().is_none());
        assert_eq!(audit_count(&pool, "delete_announcement", &announcement.id.to_string()).await, 1);
    }

    #[tokio::test]
    async fn publish_transitions_draft_and_audits() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let announcement = svc.create(actor, create_input(false)).await.unwrap();
        assert!(announcement.published_at.is_none());

        let result = svc.publish(actor, announcement.id).await.unwrap();
        assert!(result.published_at.is_some(), "publish should stamp published_at");
        assert_eq!(audit_count(&pool, "publish_announcement", &announcement.id.to_string()).await, 1);
    }

    #[tokio::test]
    async fn publish_is_idempotent_for_already_published() {
        // publish-then-publish-again: second call still updates the
        // row + writes an audit row, but the integration event should
        // not double-dispatch. We can't observe the dispatch directly
        // (no registered integrations), so we assert:
        //  - both calls return Ok
        //  - both calls write an audit row (two total)
        //  - the row remains Published throughout
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let announcement = svc.create(actor, create_input(true)).await.unwrap();
        assert!(announcement.published_at.is_some());

        let _ = svc.publish(actor, announcement.id).await.unwrap();
        let again = svc.publish(actor, announcement.id).await.unwrap();
        assert!(again.published_at.is_some());

        assert_eq!(audit_count(&pool, "publish_announcement", &announcement.id.to_string()).await, 2);
    }

    #[tokio::test]
    async fn unpublish_clears_published_at_and_audits() {
        let pool = fresh_pool().await;
        let svc = make_service(pool.clone());
        let actor = make_actor(&pool).await;

        let announcement = svc.create(actor, create_input(true)).await.unwrap();
        assert!(announcement.published_at.is_some());

        let result = svc.unpublish(actor, announcement.id).await.unwrap();
        assert!(result.published_at.is_none(), "unpublish should clear published_at");
        assert_eq!(audit_count(&pool, "unpublish_announcement", &announcement.id.to_string()).await, 1);
    }
}
