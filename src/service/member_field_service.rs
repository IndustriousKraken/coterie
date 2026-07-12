//! Org-defined member fields: definition CRUD (validated + audited)
//! and value saves (bounded, typed, member-editability enforced). See
//! the member-custom-fields capability spec.

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::{
    domain::{
        CreateMemberFieldRequest, FieldWithValue, MemberFieldDefinition, MemberFieldType,
        UpdateMemberFieldRequest,
    },
    error::{AppError, Result},
    repository::MemberFieldRepository,
    service::audit_service::AuditService,
};

/// Longest stored value. Generous for IDs/handles/links; a bound so an
/// unauthenticated-adjacent surface (member profile) can't stuff blobs.
pub const MAX_FIELD_VALUE_CHARS: usize = 500;

/// Who is saving values — members are restricted to `member_editable`
/// definitions; admins edit everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldScope {
    Admin,
    Member,
}

pub struct MemberFieldService {
    repo: Arc<dyn MemberFieldRepository>,
    audit_service: Arc<AuditService>,
}

impl MemberFieldService {
    pub fn new(repo: Arc<dyn MemberFieldRepository>, audit_service: Arc<AuditService>) -> Self {
        Self {
            repo,
            audit_service,
        }
    }

    pub async fn list_definitions(
        &self,
        include_inactive: bool,
    ) -> Result<Vec<MemberFieldDefinition>> {
        self.repo.list_definitions(include_inactive).await
    }

    pub async fn create_definition(
        &self,
        actor_id: Uuid,
        request: CreateMemberFieldRequest,
    ) -> Result<MemberFieldDefinition> {
        let name = request.name.trim();
        if name.is_empty() {
            return Err(AppError::BadRequest("Field name is required".to_string()));
        }
        if name.len() > 100 {
            return Err(AppError::BadRequest("Field name too long".to_string()));
        }
        let field_type = MemberFieldType::from_str(request.field_type.trim())
            .ok_or_else(|| AppError::BadRequest("Field type must be text or url".to_string()))?;

        let field_key = match request
            .field_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(k) => k.to_string(),
            None => slugify(name),
        };
        if field_key.is_empty() || field_key.len() > 100 || !is_valid_key(&field_key) {
            return Err(AppError::BadRequest(
                "Field key must be lowercase letters, digits and hyphens".to_string(),
            ));
        }
        if self.repo.find_definition_by_key(&field_key).await?.is_some() {
            return Err(AppError::BadRequest(format!(
                "A field with key '{}' already exists",
                field_key,
            )));
        }

        let def = MemberFieldDefinition {
            id: Uuid::new_v4(),
            name: name.to_string(),
            field_key,
            field_type,
            member_editable: request.member_editable,
            sort_order: request.sort_order,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.repo.create_definition(def.clone()).await?;

        self.audit_service
            .log(
                Some(actor_id),
                "create_member_field",
                "member_field",
                &def.id.to_string(),
                None,
                Some(&def.name),
                None,
            )
            .await;
        Ok(def)
    }

    pub async fn update_definition(
        &self,
        actor_id: Uuid,
        id: Uuid,
        request: UpdateMemberFieldRequest,
    ) -> Result<MemberFieldDefinition> {
        let mut def = self
            .repo
            .find_definition(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Field definition not found".to_string()))?;
        let before = def.clone();

        if let Some(name) = request.name {
            let name = name.trim().to_string();
            if name.is_empty() || name.len() > 100 {
                return Err(AppError::BadRequest("Invalid field name".to_string()));
            }
            def.name = name;
        }
        if let Some(t) = request.field_type {
            def.field_type = MemberFieldType::from_str(t.trim()).ok_or_else(|| {
                AppError::BadRequest("Field type must be text or url".to_string())
            })?;
        }
        if let Some(me) = request.member_editable {
            def.member_editable = me;
        }
        if let Some(so) = request.sort_order {
            def.sort_order = so;
        }
        if let Some(active) = request.is_active {
            def.is_active = active;
        }

        self.repo.update_definition(def.clone()).await?;
        self.audit_service
            .log(
                Some(actor_id),
                "update_member_field",
                "member_field",
                &id.to_string(),
                Some(&format!(
                    "{} ({}, active={})",
                    before.name,
                    before.field_type.as_str(),
                    before.is_active,
                )),
                Some(&format!(
                    "{} ({}, active={})",
                    def.name,
                    def.field_type.as_str(),
                    def.is_active,
                )),
                None,
            )
            .await;
        Ok(def)
    }

    /// Hard delete; the member values cascade with it (migration 039).
    pub async fn delete_definition(&self, actor_id: Uuid, id: Uuid) -> Result<()> {
        let def = self
            .repo
            .find_definition(id)
            .await?
            .ok_or_else(|| AppError::NotFound("Field definition not found".to_string()))?;
        self.repo.delete_definition(id).await?;
        self.audit_service
            .log(
                Some(actor_id),
                "delete_member_field",
                "member_field",
                &id.to_string(),
                Some(&def.name),
                None,
                None,
            )
            .await;
        Ok(())
    }

    /// Active definitions with the member's values, for display.
    /// `Member` scope filters to member-editable definitions.
    pub async fn fields_for(&self, member_id: Uuid, scope: FieldScope) -> Result<Vec<FieldWithValue>> {
        let mut fields = self.repo.fields_with_values(member_id).await?;
        if scope == FieldScope::Member {
            fields.retain(|f| f.definition.member_editable);
        }
        Ok(fields)
    }

    /// Save a batch of (field_key, value) pairs for a member. Every
    /// pair is validated against its definition: it must exist and be
    /// active; `Member` scope additionally requires member_editable
    /// (a crafted POST can't write locked fields). Values are trimmed
    /// and bounded; `url` fields must be http(s) when non-empty; blank
    /// clears. Validation failures reject the whole save.
    pub async fn save_values(
        &self,
        actor_id: Uuid,
        member_id: Uuid,
        pairs: &[(String, String)],
        scope: FieldScope,
    ) -> Result<()> {
        // Validate everything before writing anything.
        let mut writes: Vec<(Uuid, String)> = Vec::with_capacity(pairs.len());
        for (key, raw) in pairs {
            let def = self
                .repo
                .find_definition_by_key(key)
                .await?
                .filter(|d| d.is_active)
                .ok_or_else(|| {
                    AppError::BadRequest(format!("Unknown or inactive field '{}'", key))
                })?;
            if scope == FieldScope::Member && !def.member_editable {
                return Err(AppError::Forbidden);
            }
            let value = raw.trim();
            if value.chars().count() > MAX_FIELD_VALUE_CHARS {
                return Err(AppError::BadRequest(format!(
                    "Value for '{}' exceeds {} characters",
                    def.name, MAX_FIELD_VALUE_CHARS,
                )));
            }
            if def.field_type == MemberFieldType::Url
                && !value.is_empty()
                && !(value.starts_with("http://") || value.starts_with("https://"))
            {
                return Err(AppError::BadRequest(format!(
                    "'{}' must be a link starting with http:// or https://",
                    def.name,
                )));
            }
            writes.push((def.id, value.to_string()));
        }

        for (field_id, value) in &writes {
            self.repo.set_value(member_id, *field_id, value).await?;
        }

        self.audit_service
            .log(
                Some(actor_id),
                "update_member_fields",
                "member",
                &member_id.to_string(),
                None,
                Some(&format!("{} field(s) saved", writes.len())),
                None,
            )
            .await;
        Ok(())
    }
}

/// Lowercase the name and collapse everything non-alphanumeric to
/// single hyphens: "HackTheBox ID" → "hackthebox-id".
fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_hyphen = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_hyphen = false;
        } else if !last_hyphen {
            out.push('-');
            last_hyphen = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

fn is_valid_key(key: &str) -> bool {
    key.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::SqliteMemberFieldRepository;
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
            .expect("connect");
        sqlx::migrate!("./migrations").run(&pool).await.expect("migrate");
        pool
    }

    fn service(pool: &SqlitePool) -> MemberFieldService {
        MemberFieldService::new(
            Arc::new(SqliteMemberFieldRepository::new(pool.clone())),
            Arc::new(AuditService::new(pool.clone())),
        )
    }

    async fn make_member(pool: &SqlitePool) -> Uuid {
        use crate::repository::{MemberRepository, SqliteMemberRepository};
        let repo = SqliteMemberRepository::new(pool.clone());
        repo.create(crate::domain::CreateMemberRequest {
            email: format!("m-{}@example.com", Uuid::new_v4()),
            username: format!("u{}", Uuid::new_v4().simple()),
            full_name: "Field Tester".to_string(),
            password: "p4ssword_long_enough".to_string(),
            membership_type_id: None,
            ..Default::default()
        })
        .await
        .expect("member")
        .id
    }

    fn create_req(name: &str, field_type: &str, member_editable: bool) -> CreateMemberFieldRequest {
        CreateMemberFieldRequest {
            name: name.to_string(),
            field_key: None,
            field_type: field_type.to_string(),
            member_editable,
            sort_order: 0,
        }
    }

    #[tokio::test]
    async fn create_slugifies_and_rejects_duplicates() {
        let pool = fresh_pool().await;
        let svc = service(&pool);
        let actor = make_member(&pool).await;

        let def = svc
            .create_definition(actor, create_req("HackTheBox ID", "text", true))
            .await
            .unwrap();
        assert_eq!(def.field_key, "hackthebox-id");

        let dup = svc
            .create_definition(actor, create_req("HackTheBox ID 2", "text", true))
            .await
            .unwrap();
        assert_eq!(dup.field_key, "hackthebox-id-2");

        let mut explicit = create_req("Other", "text", true);
        explicit.field_key = Some("hackthebox-id".to_string());
        let err = svc.create_definition(actor, explicit).await;
        assert!(matches!(err, Err(AppError::BadRequest(_))), "duplicate key rejected");

        let bad_type = svc
            .create_definition(actor, create_req("Bad", "dropdown", true))
            .await;
        assert!(matches!(bad_type, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn value_rules_enforced() {
        let pool = fresh_pool().await;
        let svc = service(&pool);
        let actor = make_member(&pool).await;
        let member = make_member(&pool).await;

        svc.create_definition(actor, create_req("LinkedIn", "url", true))
            .await
            .unwrap();

        // Non-URL rejected for url type.
        let err = svc
            .save_values(
                actor,
                member,
                &[("linkedin".to_string(), "not a link".to_string())],
                FieldScope::Admin,
            )
            .await;
        assert!(matches!(err, Err(AppError::BadRequest(_))));

        // Valid URL stored; visible via fields_for.
        svc.save_values(
            actor,
            member,
            &[("linkedin".to_string(), "https://linkedin.com/in/x".to_string())],
            FieldScope::Admin,
        )
        .await
        .unwrap();
        let fields = svc.fields_for(member, FieldScope::Admin).await.unwrap();
        assert_eq!(
            fields[0].value.as_deref(),
            Some("https://linkedin.com/in/x")
        );

        // Over-long value rejected.
        let long = "h".repeat(MAX_FIELD_VALUE_CHARS + 1);
        let err = svc
            .save_values(
                actor,
                member,
                &[("linkedin".to_string(), format!("https://x.com/{long}"))],
                FieldScope::Admin,
            )
            .await;
        assert!(matches!(err, Err(AppError::BadRequest(_))));

        // Blank clears the stored row.
        svc.save_values(
            actor,
            member,
            &[("linkedin".to_string(), "  ".to_string())],
            FieldScope::Admin,
        )
        .await
        .unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM member_field_values")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0, "blank clears");

        // Unknown key rejected.
        let err = svc
            .save_values(
                actor,
                member,
                &[("nope".to_string(), "x".to_string())],
                FieldScope::Admin,
            )
            .await;
        assert!(matches!(err, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn member_scope_cannot_write_locked_fields() {
        let pool = fresh_pool().await;
        let svc = service(&pool);
        let actor = make_member(&pool).await;
        let member = make_member(&pool).await;

        svc.create_definition(actor, create_req("Committee", "text", false))
            .await
            .unwrap();

        // Member scope: field neither listed nor writable.
        let visible = svc.fields_for(member, FieldScope::Member).await.unwrap();
        assert!(visible.is_empty(), "locked field hidden from members");
        let err = svc
            .save_values(
                member,
                member,
                &[("committee".to_string(), "infra".to_string())],
                FieldScope::Member,
            )
            .await;
        assert!(matches!(err, Err(AppError::Forbidden)));

        // Admin scope writes fine.
        svc.save_values(
            actor,
            member,
            &[("committee".to_string(), "infra".to_string())],
            FieldScope::Admin,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn inactive_definition_hidden_and_unwritable_but_values_survive() {
        let pool = fresh_pool().await;
        let svc = service(&pool);
        let actor = make_member(&pool).await;
        let member = make_member(&pool).await;

        let def = svc
            .create_definition(actor, create_req("Rank", "text", true))
            .await
            .unwrap();
        svc.save_values(
            actor,
            member,
            &[("rank".to_string(), "3 dan".to_string())],
            FieldScope::Admin,
        )
        .await
        .unwrap();

        svc.update_definition(
            actor,
            def.id,
            UpdateMemberFieldRequest {
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(svc.fields_for(member, FieldScope::Admin).await.unwrap().is_empty());
        let err = svc
            .save_values(
                actor,
                member,
                &[("rank".to_string(), "4 dan".to_string())],
                FieldScope::Admin,
            )
            .await;
        assert!(matches!(err, Err(AppError::BadRequest(_))));

        // Reactivate: the stored value resurfaces unchanged.
        svc.update_definition(
            actor,
            def.id,
            UpdateMemberFieldRequest {
                is_active: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let fields = svc.fields_for(member, FieldScope::Admin).await.unwrap();
        assert_eq!(fields[0].value.as_deref(), Some("3 dan"));
    }

    #[tokio::test]
    async fn definition_delete_cascades_values() {
        let pool = fresh_pool().await;
        let svc = service(&pool);
        let actor = make_member(&pool).await;
        let member = make_member(&pool).await;

        let def = svc
            .create_definition(actor, create_req("Temp", "text", true))
            .await
            .unwrap();
        svc.save_values(
            actor,
            member,
            &[("temp".to_string(), "v".to_string())],
            FieldScope::Admin,
        )
        .await
        .unwrap();

        svc.delete_definition(actor, def.id).await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM member_field_values")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0, "values cascade with the definition");
    }
}
