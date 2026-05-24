//! Integration tests for the admin type-mutation handlers. Each of the
//! six mutations (create/update/delete × {event-type, announcement-type,
//! membership-type}) must write exactly one row to `audit_logs` with the
//! expected action / entity_type / old_value / new_value. A final test
//! confirms the fire-and-forget contract: when the `audit_logs` table
//! is unavailable (simulating a transient DB failure), the underlying
//! type mutation still commits.
//!
//! These tests exercise the handlers directly via their extractor types
//! (`State`, `Extension`, `Path`, `axum::Form`); we don't spin up a full
//! axum router because the only behavior under test is the audit-log
//! write that happens inside each handler after a successful service call.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    Extension,
};
use coterie::{
    api::{
        middleware::auth::CurrentUser,
        state::{AnnouncementBasicTypeService, EventBasicTypeService},
    },
    domain::{BasicTypeKind, CreateBasicTypeRequest, CreateMembershipTypeRequest, Member},
    repository::{
        BasicTypeRepository, MembershipTypeRepository, SqliteBasicTypeRepository,
        SqliteMembershipTypeRepository,
    },
    service::{
        audit_service::AuditService, basic_type_service::BasicTypeService,
        membership_type_service::MembershipTypeService,
    },
    web::portal::admin::types::{
        admin_create_basic_type, admin_create_membership_type, admin_delete_basic_type,
        admin_delete_membership_type, admin_update_basic_type, admin_update_membership_type,
        BasicTypeForm, MembershipTypeForm,
    },
};
use sqlx::SqlitePool;
use uuid::Uuid;

mod common;
use common::fresh_pool_no_seeded_basic_types as fresh_pool;

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct H {
    pool: SqlitePool,
    event_svc: Arc<BasicTypeService>,
    announcement_svc: Arc<BasicTypeService>,
    membership_svc: Arc<MembershipTypeService>,
    audit: Arc<AuditService>,
    current_user: CurrentUser,
}

async fn build_harness() -> H {
    let pool = fresh_pool().await.expect("fresh pool");
    let basic_repo: Arc<dyn BasicTypeRepository> =
        Arc::new(SqliteBasicTypeRepository::new(pool.clone()));
    let event_svc = Arc::new(BasicTypeService::new(
        basic_repo.clone(),
        BasicTypeKind::Event,
    ));
    let announcement_svc = Arc::new(BasicTypeService::new(
        basic_repo,
        BasicTypeKind::Announcement,
    ));
    let membership_repo: Arc<dyn MembershipTypeRepository> =
        Arc::new(SqliteMembershipTypeRepository::new(pool.clone()));
    let membership_svc = Arc::new(MembershipTypeService::new(membership_repo));
    let audit = Arc::new(AuditService::new(pool.clone()));

    let member_id = common::make_member(&pool).await;
    let member = fetch_member(&pool, member_id).await;
    let current_user = CurrentUser { member };

    H {
        pool,
        event_svc,
        announcement_svc,
        membership_svc,
        audit,
        current_user,
    }
}

async fn fetch_member(pool: &SqlitePool, id: Uuid) -> Member {
    use coterie::repository::{MemberRepository, SqliteMemberRepository};
    let repo = SqliteMemberRepository::new(pool.clone());
    MemberRepository::find_by_id(&repo, id)
        .await
        .expect("find_by_id")
        .expect("member exists")
}

#[derive(Debug, Clone)]
struct AuditRow {
    action: String,
    entity_type: String,
    entity_id: String,
    old_value: Option<String>,
    new_value: Option<String>,
    actor_id: Option<String>,
}

async fn fetch_audit_rows(pool: &SqlitePool, entity_type: &str, entity_id: Uuid) -> Vec<AuditRow> {
    sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT action, entity_type, entity_id, old_value, new_value, actor_id \
         FROM audit_logs WHERE entity_type = ? AND entity_id = ?",
    )
    .bind(entity_type)
    .bind(entity_id.to_string())
    .fetch_all(pool)
    .await
    .expect("query audit_logs")
    .into_iter()
    .map(
        |(action, entity_type, entity_id, old_value, new_value, actor_id)| AuditRow {
            action,
            entity_type,
            entity_id,
            old_value,
            new_value,
            actor_id,
        },
    )
    .collect()
}

fn basic_form(name: &str) -> BasicTypeForm {
    BasicTypeForm {
        name: name.to_string(),
        slug: None,
        description: None,
        color: None,
        icon: None,
        is_active: Some("on".to_string()),
    }
}

fn membership_form(name: &str) -> MembershipTypeForm {
    MembershipTypeForm {
        name: name.to_string(),
        slug: None,
        description: None,
        color: None,
        icon: None,
        fee_dollars: "10.00".to_string(),
        billing_period: "monthly".to_string(),
        is_active: Some("on".to_string()),
    }
}

async fn type_id_for(svc: &BasicTypeService, name: &str) -> Uuid {
    svc.list(true)
        .await
        .expect("list")
        .into_iter()
        .find(|t| t.name == name)
        .expect("type exists")
        .id
}

async fn membership_id_for(svc: &MembershipTypeService, name: &str) -> Uuid {
    svc.list(true)
        .await
        .expect("list")
        .into_iter()
        .find(|t| t.name == name)
        .expect("membership type exists")
        .id
}

// ---------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------

#[tokio::test]
async fn create_event_type_writes_audit_row() {
    let h = build_harness().await;

    let _ = admin_create_basic_type(
        State(EventBasicTypeService(h.event_svc.clone())),
        State(AnnouncementBasicTypeService(h.announcement_svc.clone())),
        State(h.audit.clone()),
        Extension(h.current_user.clone()),
        Path("event".to_string()),
        axum::Form(basic_form("Workshop")),
    )
    .await;

    let id = type_id_for(&h.event_svc, "Workshop").await;
    let rows = fetch_audit_rows(&h.pool, "event_type", id).await;
    assert_eq!(
        rows.len(),
        1,
        "expected one audit row for create_event_type"
    );
    let row = &rows[0];
    assert_eq!(row.action, "create_event_type");
    assert_eq!(row.entity_type, "event_type");
    assert_eq!(row.entity_id, id.to_string());
    assert_eq!(row.old_value, None);
    assert_eq!(row.new_value.as_deref(), Some("Workshop"));
    assert_eq!(
        row.actor_id.as_deref(),
        Some(h.current_user.member.id.to_string().as_str())
    );
}

#[tokio::test]
async fn update_event_type_writes_audit_row_with_old_and_new_names() {
    let h = build_harness().await;

    let created = h
        .event_svc
        .create(CreateBasicTypeRequest {
            name: "Workshop".to_string(),
            slug: Some("workshop".to_string()),
            description: None,
            color: None,
            icon: None,
        })
        .await
        .expect("seed event type");

    let mut form = basic_form("Tournament");
    form.slug = None;
    let _ = admin_update_basic_type(
        State(EventBasicTypeService(h.event_svc.clone())),
        State(AnnouncementBasicTypeService(h.announcement_svc.clone())),
        State(h.audit.clone()),
        Extension(h.current_user.clone()),
        Path(("event".to_string(), created.id.to_string())),
        axum::Form(form),
    )
    .await;

    let rows = fetch_audit_rows(&h.pool, "event_type", created.id).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.action, "update_event_type");
    assert_eq!(row.old_value.as_deref(), Some("Workshop"));
    assert_eq!(row.new_value.as_deref(), Some("Tournament"));
}

#[tokio::test]
async fn delete_event_type_writes_audit_row_with_old_name() {
    let h = build_harness().await;

    let created = h
        .event_svc
        .create(CreateBasicTypeRequest {
            name: "Workshop".to_string(),
            slug: Some("workshop".to_string()),
            description: None,
            color: None,
            icon: None,
        })
        .await
        .expect("seed event type");

    let _ = admin_delete_basic_type(
        State(EventBasicTypeService(h.event_svc.clone())),
        State(AnnouncementBasicTypeService(h.announcement_svc.clone())),
        State(h.audit.clone()),
        Extension(h.current_user.clone()),
        Path(("event".to_string(), created.id.to_string())),
    )
    .await;

    let rows = fetch_audit_rows(&h.pool, "event_type", created.id).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.action, "delete_event_type");
    assert_eq!(row.old_value.as_deref(), Some("Workshop"));
    assert_eq!(row.new_value, None);
}

// ---------------------------------------------------------------------
// Announcement types
// ---------------------------------------------------------------------

#[tokio::test]
async fn create_announcement_type_writes_audit_row() {
    let h = build_harness().await;

    let _ = admin_create_basic_type(
        State(EventBasicTypeService(h.event_svc.clone())),
        State(AnnouncementBasicTypeService(h.announcement_svc.clone())),
        State(h.audit.clone()),
        Extension(h.current_user.clone()),
        Path("announcement".to_string()),
        axum::Form(basic_form("Newsletter")),
    )
    .await;

    let id = type_id_for(&h.announcement_svc, "Newsletter").await;
    let rows = fetch_audit_rows(&h.pool, "announcement_type", id).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.action, "create_announcement_type");
    assert_eq!(row.entity_type, "announcement_type");
    assert_eq!(row.old_value, None);
    assert_eq!(row.new_value.as_deref(), Some("Newsletter"));
}

#[tokio::test]
async fn update_announcement_type_writes_audit_row() {
    let h = build_harness().await;

    let created = h
        .announcement_svc
        .create(CreateBasicTypeRequest {
            name: "Newsletter".to_string(),
            slug: Some("newsletter".to_string()),
            description: None,
            color: None,
            icon: None,
        })
        .await
        .expect("seed announcement type");

    let _ = admin_update_basic_type(
        State(EventBasicTypeService(h.event_svc.clone())),
        State(AnnouncementBasicTypeService(h.announcement_svc.clone())),
        State(h.audit.clone()),
        Extension(h.current_user.clone()),
        Path(("announcement".to_string(), created.id.to_string())),
        axum::Form(basic_form("Bulletin")),
    )
    .await;

    let rows = fetch_audit_rows(&h.pool, "announcement_type", created.id).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.action, "update_announcement_type");
    assert_eq!(row.old_value.as_deref(), Some("Newsletter"));
    assert_eq!(row.new_value.as_deref(), Some("Bulletin"));
}

#[tokio::test]
async fn delete_announcement_type_writes_audit_row() {
    let h = build_harness().await;

    let created = h
        .announcement_svc
        .create(CreateBasicTypeRequest {
            name: "Newsletter".to_string(),
            slug: Some("newsletter".to_string()),
            description: None,
            color: None,
            icon: None,
        })
        .await
        .expect("seed announcement type");

    let _ = admin_delete_basic_type(
        State(EventBasicTypeService(h.event_svc.clone())),
        State(AnnouncementBasicTypeService(h.announcement_svc.clone())),
        State(h.audit.clone()),
        Extension(h.current_user.clone()),
        Path(("announcement".to_string(), created.id.to_string())),
    )
    .await;

    let rows = fetch_audit_rows(&h.pool, "announcement_type", created.id).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.action, "delete_announcement_type");
    assert_eq!(row.old_value.as_deref(), Some("Newsletter"));
    assert_eq!(row.new_value, None);
}

// ---------------------------------------------------------------------
// Membership types
// ---------------------------------------------------------------------

#[tokio::test]
async fn create_membership_type_writes_audit_row() {
    let h = build_harness().await;

    let _ = admin_create_membership_type(
        State(h.membership_svc.clone()),
        State(h.audit.clone()),
        Extension(h.current_user.clone()),
        axum::Form(membership_form("Annual")),
    )
    .await;

    let id = membership_id_for(&h.membership_svc, "Annual").await;
    let rows = fetch_audit_rows(&h.pool, "membership_type", id).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.action, "create_membership_type");
    assert_eq!(row.entity_type, "membership_type");
    assert_eq!(row.old_value, None);
    assert_eq!(row.new_value.as_deref(), Some("Annual"));
}

#[tokio::test]
async fn update_membership_type_writes_audit_row() {
    let h = build_harness().await;

    let created = h
        .membership_svc
        .create(CreateMembershipTypeRequest {
            name: "Annual".to_string(),
            slug: Some("annual".to_string()),
            description: None,
            color: None,
            icon: None,
            fee_cents: 1000,
            billing_period: "monthly".to_string(),
        })
        .await
        .expect("seed membership type");

    let _ = admin_update_membership_type(
        State(h.membership_svc.clone()),
        State(h.audit.clone()),
        Extension(h.current_user.clone()),
        Path(created.id.to_string()),
        axum::Form(membership_form("Premium")),
    )
    .await;

    let rows = fetch_audit_rows(&h.pool, "membership_type", created.id).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.action, "update_membership_type");
    assert_eq!(row.old_value.as_deref(), Some("Annual"));
    assert_eq!(row.new_value.as_deref(), Some("Premium"));
}

#[tokio::test]
async fn delete_membership_type_writes_audit_row() {
    let h = build_harness().await;

    let created = h
        .membership_svc
        .create(CreateMembershipTypeRequest {
            name: "Annual".to_string(),
            slug: Some("annual".to_string()),
            description: None,
            color: None,
            icon: None,
            fee_cents: 1000,
            billing_period: "monthly".to_string(),
        })
        .await
        .expect("seed membership type");

    let _ = admin_delete_membership_type(
        State(h.membership_svc.clone()),
        State(h.audit.clone()),
        Extension(h.current_user.clone()),
        Path(created.id.to_string()),
    )
    .await;

    let rows = fetch_audit_rows(&h.pool, "membership_type", created.id).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.action, "delete_membership_type");
    assert_eq!(row.old_value.as_deref(), Some("Annual"));
    assert_eq!(row.new_value, None);
}

// ---------------------------------------------------------------------
// Fire-and-forget contract: audit failure does not roll back the mutation
// ---------------------------------------------------------------------

#[tokio::test]
async fn type_mutation_commits_when_audit_insert_fails() {
    let h = build_harness().await;

    // Simulate a misbehaving audit table by dropping it. The audit
    // service's INSERT will fail; per the fire-and-forget contract the
    // handler must still complete the type mutation.
    sqlx::query("DROP TABLE audit_logs")
        .execute(&h.pool)
        .await
        .expect("drop audit_logs");

    let _ = admin_create_basic_type(
        State(EventBasicTypeService(h.event_svc.clone())),
        State(AnnouncementBasicTypeService(h.announcement_svc.clone())),
        State(h.audit.clone()),
        Extension(h.current_user.clone()),
        Path("event".to_string()),
        axum::Form(basic_form("Workshop")),
    )
    .await;

    // The mutation must still have happened despite the audit failure.
    let exists = h
        .event_svc
        .list(true)
        .await
        .expect("list")
        .into_iter()
        .any(|t| t.name == "Workshop");
    assert!(
        exists,
        "type mutation should commit even when audit insert errors"
    );
}
