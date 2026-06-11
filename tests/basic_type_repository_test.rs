//! Integration tests for the unified `BasicTypeRepository` / `BasicTypeService`.
//! These cover the kind-discriminated paths the consolidation introduces:
//! the delete-error message includes the right display name, and a service
//! instance only sees rows from its own kind's table.

use std::sync::Arc;

use coterie::{
    domain::{BasicTypeKind, CreateBasicTypeRequest, CreateMemberRequest},
    repository::{
        BasicTypeRepository, MemberRepository, SqliteBasicTypeRepository, SqliteMemberRepository,
    },
    service::basic_type_service::BasicTypeService,
};
use uuid::Uuid;

mod common;
use common::fresh_pool_no_seeded_basic_types as fresh_pool;

#[tokio::test]
async fn delete_in_use_event_type_returns_event_type_display_name() -> anyhow::Result<()> {
    let pool = fresh_pool().await?;
    let repo: Arc<dyn BasicTypeRepository> = Arc::new(SqliteBasicTypeRepository::new(pool.clone()));
    let service = BasicTypeService::new(repo.clone(), BasicTypeKind::Event);

    let created = service
        .create(CreateBasicTypeRequest {
            name: "Workshop".to_string(),
            slug: Some("workshop".to_string()),
            description: None,
            color: Some("#2196F3".to_string()),
            icon: None,
        })
        .await?;

    // Seed a member to satisfy events.created_by FK, then an event using
    // this type so usage_count > 0.
    let member_repo = SqliteMemberRepository::new(pool.clone());
    let member = member_repo
        .create(CreateMemberRequest {
            email: "owner@example.com".to_string(),
            username: "owner".to_string(),
            full_name: "Owner".to_string(),
            password: "very-long-password-123".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await?;

    let event_id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO events (
            id, title, description, event_type, event_type_id,
            visibility, start_time, created_by, created_at, updated_at
        ) VALUES (?, 'Test', '', 'Workshop', ?, 'MembersOnly',
                  CURRENT_TIMESTAMP, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
        "#,
    )
    .bind(&event_id)
    .bind(created.id.to_string())
    .bind(member.id.to_string())
    .execute(&pool)
    .await?;

    let err = service
        .delete(created.id)
        .await
        .expect_err("delete with usage should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("event type"),
        "expected 'event type' in error message, got: {}",
        msg
    );
    assert!(
        !msg.contains("announcement type"),
        "should not mention 'announcement type' for kind=Event, got: {}",
        msg
    );

    Ok(())
}

#[tokio::test]
async fn delete_in_use_announcement_type_returns_announcement_type_display_name(
) -> anyhow::Result<()> {
    let pool = fresh_pool().await?;
    let repo: Arc<dyn BasicTypeRepository> = Arc::new(SqliteBasicTypeRepository::new(pool.clone()));
    let service = BasicTypeService::new(repo.clone(), BasicTypeKind::Announcement);

    let created = service
        .create(CreateBasicTypeRequest {
            name: "News".to_string(),
            slug: Some("news".to_string()),
            description: None,
            color: Some("#607D8B".to_string()),
            icon: None,
        })
        .await?;

    let member_repo = SqliteMemberRepository::new(pool.clone());
    let member = member_repo
        .create(CreateMemberRequest {
            email: "ann@example.com".to_string(),
            username: "ann".to_string(),
            full_name: "Ann Owner".to_string(),
            password: "very-long-password-123".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await?;

    let ann_id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO announcements (
            id, title, content, announcement_type, announcement_type_id,
            is_public, featured, created_by, created_at, updated_at
        ) VALUES (?, 'Hello', 'body', 'News', ?, 0, 0, ?,
                  CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
        "#,
    )
    .bind(&ann_id)
    .bind(created.id.to_string())
    .bind(member.id.to_string())
    .execute(&pool)
    .await?;

    let err = service
        .delete(created.id)
        .await
        .expect_err("delete with usage should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("announcement type"),
        "expected 'announcement type' in error message, got: {}",
        msg
    );

    Ok(())
}

#[tokio::test]
async fn list_event_and_announcement_kinds_query_disjoint_tables() -> anyhow::Result<()> {
    let pool = fresh_pool().await?;
    let repo: Arc<dyn BasicTypeRepository> = Arc::new(SqliteBasicTypeRepository::new(pool.clone()));
    let event_service = BasicTypeService::new(repo.clone(), BasicTypeKind::Event);
    let announcement_service = BasicTypeService::new(repo.clone(), BasicTypeKind::Announcement);

    let event_a = event_service
        .create(CreateBasicTypeRequest {
            name: "Workshop".to_string(),
            slug: Some("workshop".to_string()),
            description: None,
            color: None,
            icon: None,
        })
        .await?;
    let event_b = event_service
        .create(CreateBasicTypeRequest {
            name: "Social".to_string(),
            slug: Some("social".to_string()),
            description: None,
            color: None,
            icon: None,
        })
        .await?;
    let ann_a = announcement_service
        .create(CreateBasicTypeRequest {
            name: "News".to_string(),
            slug: Some("news".to_string()),
            description: None,
            color: None,
            icon: None,
        })
        .await?;

    let events = event_service.list(false).await?;
    let announcements = announcement_service.list(false).await?;

    let event_ids: Vec<_> = events.iter().map(|t| t.id).collect();
    let ann_ids: Vec<_> = announcements.iter().map(|t| t.id).collect();

    assert!(event_ids.contains(&event_a.id));
    assert!(event_ids.contains(&event_b.id));
    assert!(!event_ids.contains(&ann_a.id));

    assert!(ann_ids.contains(&ann_a.id));
    assert!(!ann_ids.contains(&event_a.id));
    assert!(!ann_ids.contains(&event_b.id));

    // The two queries hit physically separate tables; sets must be disjoint.
    for e in &event_ids {
        assert!(
            !ann_ids.contains(e),
            "event id {} bled into announcements",
            e
        );
    }

    Ok(())
}
