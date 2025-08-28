use async_trait::async_trait;
use chrono::{DateTime, Utc, NaiveDateTime};
use sqlx::{SqlitePool, FromRow};
use uuid::Uuid;

use crate::{
    domain::{Member, MemberStatus, MembershipType, CreateMemberRequest, UpdateMemberRequest},
    error::{AppError, Result},
    repository::MemberRepository,
};

// Database row struct that matches SQLite schema
#[derive(FromRow)]
struct MemberRow {
    id: String,
    email: String,
    username: String,
    full_name: String,
    status: String,
    membership_type: String,
    joined_at: NaiveDateTime,
    expires_at: Option<NaiveDateTime>,
    dues_paid_until: Option<NaiveDateTime>,
    bypass_dues: i32,
    notes: Option<String>,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

pub struct SqliteMemberRepository {
    pool: SqlitePool,
}

impl SqliteMemberRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_member(row: MemberRow) -> Result<Member> {
        Ok(Member {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            email: row.email,
            username: row.username,
            full_name: row.full_name,
            status: Self::parse_member_status(&row.status)?,
            membership_type: Self::parse_membership_type(&row.membership_type)?,
            joined_at: DateTime::from_naive_utc_and_offset(row.joined_at, Utc),
            expires_at: row.expires_at.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            dues_paid_until: row.dues_paid_until.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            bypass_dues: row.bypass_dues != 0,
            notes: row.notes,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }

    fn parse_member_status(s: &str) -> Result<MemberStatus> {
        match s {
            "Pending" => Ok(MemberStatus::Pending),
            "Active" => Ok(MemberStatus::Active),
            "Expired" => Ok(MemberStatus::Expired),
            "Suspended" => Ok(MemberStatus::Suspended),
            "Honorary" => Ok(MemberStatus::Honorary),
            _ => Err(AppError::Database(format!("Invalid member status: {}", s))),
        }
    }

    fn member_status_to_str(status: &MemberStatus) -> &'static str {
        match status {
            MemberStatus::Pending => "Pending",
            MemberStatus::Active => "Active",
            MemberStatus::Expired => "Expired",
            MemberStatus::Suspended => "Suspended",
            MemberStatus::Honorary => "Honorary",
        }
    }

    fn parse_membership_type(s: &str) -> Result<MembershipType> {
        match s {
            "Regular" => Ok(MembershipType::Regular),
            "Student" => Ok(MembershipType::Student),
            "Corporate" => Ok(MembershipType::Corporate),
            "Lifetime" => Ok(MembershipType::Lifetime),
            _ => Err(AppError::Database(format!("Invalid membership type: {}", s))),
        }
    }

    fn membership_type_to_str(membership_type: &MembershipType) -> &'static str {
        match membership_type {
            MembershipType::Regular => "Regular",
            MembershipType::Student => "Student",
            MembershipType::Corporate => "Corporate",
            MembershipType::Lifetime => "Lifetime",
        }
    }
}

#[async_trait]
impl MemberRepository for SqliteMemberRepository {
    async fn create(&self, request: CreateMemberRequest) -> Result<Member> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let status = MemberStatus::Pending;
        
        // Hash the password with argon2
        use argon2::{Argon2, PasswordHasher};
        use argon2::password_hash::{SaltString, rand_core::OsRng};
        
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        
        let password_hash = argon2
            .hash_password(request.password.as_bytes(), &salt)
            .map_err(|e| AppError::Database(e.to_string()))?
            .to_string();

        let status_str = Self::member_status_to_str(&status);
        let membership_type_str = Self::membership_type_to_str(&request.membership_type);
        let id_str = id.to_string();
        let now_naive = now.naive_utc();

        sqlx::query(
            r#"
            INSERT INTO members (
                id, email, username, full_name, password_hash,
                status, membership_type, joined_at, bypass_dues,
                created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&id_str)
        .bind(&request.email)
        .bind(&request.username)
        .bind(&request.full_name)
        .bind(&password_hash)
        .bind(status_str)
        .bind(membership_type_str)
        .bind(now_naive)
        .bind(0i32)  // bypass_dues as integer (0 = false)
        .bind(now_naive)
        .bind(now_naive)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve created member".to_string())
        })
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Member>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT id, email, username, full_name, status, membership_type,
                   joined_at, expires_at, dues_paid_until, bypass_dues, notes,
                   created_at, updated_at
            FROM members
            WHERE id = ?
            "#
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_member(r)?)),
            None => Ok(None)
        }
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<Member>> {
        let row = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT id, email, username, full_name, status, membership_type,
                   joined_at, expires_at, dues_paid_until, bypass_dues, notes,
                   created_at, updated_at
            FROM members
            WHERE email = ?
            "#
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_member(r)?)),
            None => Ok(None)
        }
    }

    async fn find_by_username(&self, username: &str) -> Result<Option<Member>> {
        let row = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT id, email, username, full_name, status, membership_type,
                   joined_at, expires_at, dues_paid_until, bypass_dues, notes,
                   created_at, updated_at
            FROM members
            WHERE username = ?
            "#
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_member(r)?)),
            None => Ok(None)
        }
    }

    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<Member>> {
        let rows = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT id, email, username, full_name, status, membership_type,
                   joined_at, expires_at, dues_paid_until, bypass_dues, notes,
                   created_at, updated_at
            FROM members
            ORDER BY created_at DESC
            LIMIT ? OFFSET ?
            "#
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_member)
            .collect()
    }

    async fn list_active(&self) -> Result<Vec<Member>> {
        let active_status = Self::member_status_to_str(&MemberStatus::Active);
        
        let rows = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT id, email, username, full_name, status, membership_type,
                   joined_at, expires_at, dues_paid_until, bypass_dues, notes,
                   created_at, updated_at
            FROM members
            WHERE status = ?
            ORDER BY joined_at DESC
            "#
        )
        .bind(active_status)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_member)
            .collect()
    }

    async fn list_expired(&self) -> Result<Vec<Member>> {
        let expired_status = Self::member_status_to_str(&MemberStatus::Expired);
        
        let rows = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT id, email, username, full_name, status, membership_type,
                   joined_at, expires_at, dues_paid_until, bypass_dues, notes,
                   created_at, updated_at
            FROM members
            WHERE status = ?
            ORDER BY expires_at DESC
            "#
        )
        .bind(expired_status)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter()
            .map(Self::row_to_member)
            .collect()
    }

    async fn update(&self, id: Uuid, update: UpdateMemberRequest) -> Result<Member> {
        let existing = self.find_by_id(id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        let now = Utc::now();

        let status_str = if let Some(status) = &update.status {
            Self::member_status_to_str(status)
        } else {
            Self::member_status_to_str(&existing.status)
        };

        let membership_type_str = if let Some(membership_type) = &update.membership_type {
            Self::membership_type_to_str(membership_type)
        } else {
            Self::membership_type_to_str(&existing.membership_type)
        };

        let id_str = id.to_string();
        let now_naive = now.naive_utc();
        let expires_at_naive = update.expires_at.map(|dt| dt.naive_utc());
        let bypass_dues_int = update.bypass_dues.map(|b| if b { 1i32 } else { 0i32 });

        sqlx::query(
            r#"
            UPDATE members
            SET full_name = COALESCE(?, full_name),
                status = ?,
                membership_type = ?,
                expires_at = COALESCE(?, expires_at),
                bypass_dues = COALESCE(?, bypass_dues),
                notes = COALESCE(?, notes),
                updated_at = ?
            WHERE id = ?
            "#
        )
        .bind(&update.full_name)
        .bind(status_str)
        .bind(membership_type_str)
        .bind(expires_at_naive)
        .bind(bypass_dues_int)
        .bind(&update.notes)
        .bind(now_naive)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve updated member".to_string())
        })
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        sqlx::query("DELETE FROM members WHERE id = ?")
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }
}