//! Shared test fixtures for the per-submodule `tests` blocks.
//! Each submodule reaches in via `use super::super::test_helpers::*;`
//! so the helpers live in one place rather than duplicated.

#![cfg(test)]

use std::sync::Arc;

use sqlx::{Executor, SqlitePool};
use uuid::Uuid;

use crate::{
    auth::{AuthService, SecretCrypto},
    domain::{CreateMemberRequest, Member},
    email::{EmailSender, LogSender},
    integrations::IntegrationManager,
    repository::{MemberRepository, SqliteMemberRepository, SqliteMembershipTypeRepository},
    service::{
        audit_service::AuditService, member_service::MemberService,
        membership_type_service::MembershipTypeService, settings_service::SettingsService,
    },
};

pub async fn fresh_pool() -> SqlitePool {
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

pub fn make_service(pool: SqlitePool) -> MemberService {
    let member_repo: Arc<dyn MemberRepository> =
        Arc::new(SqliteMemberRepository::new(pool.clone()));
    let auth_service = Arc::new(AuthService::new(pool.clone(), "test-secret".to_string()));
    let audit_service = Arc::new(AuditService::new(pool.clone()));
    let integration_manager = Arc::new(IntegrationManager::new());
    let email_sender: Arc<dyn EmailSender> = Arc::new(LogSender::new(
        "test@example.com".to_string(),
        "Test".to_string(),
    ));
    let membership_type_repo = Arc::new(SqliteMembershipTypeRepository::new(pool.clone()));
    let membership_type_service = Arc::new(MembershipTypeService::new(membership_type_repo));
    let crypto = Arc::new(SecretCrypto::new("test-secret-please-ignore"));
    let settings_service = Arc::new(SettingsService::new(pool.clone(), crypto));

    MemberService::new(
        member_repo,
        auth_service,
        audit_service,
        integration_manager,
        email_sender,
        membership_type_service,
        settings_service,
        pool.clone(),
        "http://test.local".to_string(),
    )
}

pub async fn make_member(pool: &SqlitePool, email: &str, username: &str) -> Member {
    let repo = SqliteMemberRepository::new(pool.clone());
    repo.create(CreateMemberRequest {
        email: email.to_string(),
        username: username.to_string(),
        full_name: "Test User".to_string(),
        password: "secure_password123".to_string(),
        membership_type_id: None,
        ..Default::default()
    })
    .await
    .unwrap()
}

pub async fn audit_count(pool: &SqlitePool, action: &str, entity_id: &Uuid) -> i64 {
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = ? AND entity_id = ?")
            .bind(action)
            .bind(entity_id.to_string())
            .fetch_one(pool)
            .await
            .unwrap();
    count.0
}
