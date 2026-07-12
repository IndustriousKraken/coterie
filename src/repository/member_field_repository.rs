//! Persistence for org-defined member fields (member-custom-fields
//! capability): definition CRUD + per-member value upsert/clear.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{FieldWithValue, MemberFieldDefinition, MemberFieldType},
    error::{AppError, Result},
};

#[async_trait]
pub trait MemberFieldRepository: Send + Sync {
    async fn list_definitions(&self, include_inactive: bool)
        -> Result<Vec<MemberFieldDefinition>>;
    async fn find_definition(&self, id: Uuid) -> Result<Option<MemberFieldDefinition>>;
    async fn find_definition_by_key(&self, key: &str) -> Result<Option<MemberFieldDefinition>>;
    async fn create_definition(&self, def: MemberFieldDefinition) -> Result<()>;
    async fn update_definition(&self, def: MemberFieldDefinition) -> Result<()>;
    /// Hard delete; stored values cascade (see migration 039).
    async fn delete_definition(&self, id: Uuid) -> Result<()>;
    /// Every ACTIVE definition (sort_order) LEFT-JOINed with this
    /// member's values. The display shape for both value-editing UIs.
    async fn fields_with_values(&self, member_id: Uuid) -> Result<Vec<FieldWithValue>>;
    /// Upsert a value; a blank (empty after trim) value deletes the
    /// row instead — "unset" is the absence of a row, never "".
    async fn set_value(&self, member_id: Uuid, field_id: Uuid, value: &str) -> Result<()>;
}

#[derive(FromRow)]
struct DefinitionRow {
    id: String,
    name: String,
    field_key: String,
    field_type: String,
    member_editable: i32,
    sort_order: i32,
    is_active: i32,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

fn row_to_definition(row: DefinitionRow) -> Result<MemberFieldDefinition> {
    Ok(MemberFieldDefinition {
        id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
        name: row.name,
        field_key: row.field_key,
        field_type: MemberFieldType::from_str(&row.field_type).unwrap_or(MemberFieldType::Text),
        member_editable: row.member_editable != 0,
        sort_order: row.sort_order,
        is_active: row.is_active != 0,
        created_at: DateTime::<Utc>::from_naive_utc_and_offset(row.created_at, Utc),
        updated_at: DateTime::<Utc>::from_naive_utc_and_offset(row.updated_at, Utc),
    })
}

pub struct SqliteMemberFieldRepository {
    pool: SqlitePool,
}

impl SqliteMemberFieldRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

const DEFINITION_COLUMNS: &str = "id, name, field_key, field_type, member_editable, \
     sort_order, is_active, created_at, updated_at";

#[async_trait]
impl MemberFieldRepository for SqliteMemberFieldRepository {
    async fn list_definitions(
        &self,
        include_inactive: bool,
    ) -> Result<Vec<MemberFieldDefinition>> {
        let sql = if include_inactive {
            format!(
                "SELECT {DEFINITION_COLUMNS} FROM member_field_definitions \
                 ORDER BY sort_order ASC, name ASC"
            )
        } else {
            format!(
                "SELECT {DEFINITION_COLUMNS} FROM member_field_definitions \
                 WHERE is_active = 1 ORDER BY sort_order ASC, name ASC"
            )
        };
        let rows = sqlx::query_as::<_, DefinitionRow>(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;
        rows.into_iter().map(row_to_definition).collect()
    }

    async fn find_definition(&self, id: Uuid) -> Result<Option<MemberFieldDefinition>> {
        let sql =
            format!("SELECT {DEFINITION_COLUMNS} FROM member_field_definitions WHERE id = ?");
        let row = sqlx::query_as::<_, DefinitionRow>(&sql)
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;
        row.map(row_to_definition).transpose()
    }

    async fn find_definition_by_key(&self, key: &str) -> Result<Option<MemberFieldDefinition>> {
        let sql = format!(
            "SELECT {DEFINITION_COLUMNS} FROM member_field_definitions WHERE field_key = ?"
        );
        let row = sqlx::query_as::<_, DefinitionRow>(&sql)
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;
        row.map(row_to_definition).transpose()
    }

    async fn create_definition(&self, def: MemberFieldDefinition) -> Result<()> {
        sqlx::query(
            "INSERT INTO member_field_definitions \
             (id, name, field_key, field_type, member_editable, sort_order, is_active, \
              created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(def.id.to_string())
        .bind(&def.name)
        .bind(&def.field_key)
        .bind(def.field_type.as_str())
        .bind(def.member_editable as i32)
        .bind(def.sort_order)
        .bind(def.is_active as i32)
        .bind(def.created_at.naive_utc())
        .bind(def.updated_at.naive_utc())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn update_definition(&self, def: MemberFieldDefinition) -> Result<()> {
        let res = sqlx::query(
            "UPDATE member_field_definitions \
             SET name = ?, field_type = ?, member_editable = ?, sort_order = ?, \
                 is_active = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&def.name)
        .bind(def.field_type.as_str())
        .bind(def.member_editable as i32)
        .bind(def.sort_order)
        .bind(def.is_active as i32)
        .bind(Utc::now().naive_utc())
        .bind(def.id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        if res.rows_affected() == 0 {
            return Err(AppError::NotFound("Field definition not found".to_string()));
        }
        Ok(())
    }

    async fn delete_definition(&self, id: Uuid) -> Result<()> {
        let res = sqlx::query("DELETE FROM member_field_definitions WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        if res.rows_affected() == 0 {
            return Err(AppError::NotFound("Field definition not found".to_string()));
        }
        Ok(())
    }

    async fn fields_with_values(&self, member_id: Uuid) -> Result<Vec<FieldWithValue>> {
        let sql = format!(
            "SELECT d.id, d.name, d.field_key, d.field_type, d.member_editable, \
                    d.sort_order, d.is_active, d.created_at, d.updated_at, v.value \
             FROM member_field_definitions d \
             LEFT JOIN member_field_values v \
                    ON v.field_id = d.id AND v.member_id = ? \
             WHERE d.is_active = 1 \
             ORDER BY d.sort_order ASC, d.name ASC"
        );
        // Suppress the unused interpolation warning shape — the SELECT
        // list is explicit here because of the joined `v.value`.
        let _ = DEFINITION_COLUMNS;

        #[derive(FromRow)]
        struct JoinedRow {
            id: String,
            name: String,
            field_key: String,
            field_type: String,
            member_editable: i32,
            sort_order: i32,
            is_active: i32,
            created_at: NaiveDateTime,
            updated_at: NaiveDateTime,
            value: Option<String>,
        }

        let rows = sqlx::query_as::<_, JoinedRow>(&sql)
            .bind(member_id.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

        rows.into_iter()
            .map(|r| {
                let value = r.value.clone();
                let definition = row_to_definition(DefinitionRow {
                    id: r.id,
                    name: r.name,
                    field_key: r.field_key,
                    field_type: r.field_type,
                    member_editable: r.member_editable,
                    sort_order: r.sort_order,
                    is_active: r.is_active,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                })?;
                Ok(FieldWithValue { definition, value })
            })
            .collect()
    }

    async fn set_value(&self, member_id: Uuid, field_id: Uuid, value: &str) -> Result<()> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            sqlx::query("DELETE FROM member_field_values WHERE member_id = ? AND field_id = ?")
                .bind(member_id.to_string())
                .bind(field_id.to_string())
                .execute(&self.pool)
                .await
                .map_err(AppError::Database)?;
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO member_field_values (member_id, field_id, value, updated_at) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT (member_id, field_id) DO UPDATE \
               SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(member_id.to_string())
        .bind(field_id.to_string())
        .bind(trimmed)
        .bind(Utc::now().naive_utc())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }
}
