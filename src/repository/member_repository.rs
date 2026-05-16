use async_trait::async_trait;
use chrono::{DateTime, Utc, NaiveDateTime};
use sqlx::{SqlitePool, FromRow};
use uuid::Uuid;

use crate::{
    domain::{Member, MemberStatus, CreateMemberRequest, UpdateMemberRequest, BillingMode},
    error::{AppError, Result},
};

/// Inputs for `MemberRepository::search`. Strongly-typed so the
/// caller can't pass an unknown sort field and the impl can map
/// `MemberSortField` → column name in one place (no string-debug
/// sort keys, no SQL injection risk).
#[derive(Debug, Clone)]
pub struct MemberQuery {
    /// Case-insensitive substring match on `full_name`, `email`, and
    /// `username`. `None` or empty string skips the filter.
    pub search: Option<String>,
    /// Filter to exactly one status. `None` skips the filter.
    pub status: Option<crate::domain::MemberStatus>,
    /// Filter to exactly one membership type by FK. `None` skips.
    pub membership_type_id: Option<Uuid>,
    pub sort: MemberSortField,
    pub order: SortOrder,
    pub limit: i64,
    pub offset: i64,
}

#[derive(Debug, Clone, Copy)]
pub enum MemberSortField {
    Name,
    Status,
    MembershipType,
    Joined,
    DuesPaidUntil,
}

#[derive(Debug, Clone, Copy)]
pub enum SortOrder {
    Asc,
    Desc,
}

#[async_trait]
pub trait MemberRepository: Send + Sync {
    async fn create(&self, member: CreateMemberRequest) -> Result<Member>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Member>>;
    async fn find_by_email(&self, email: &str) -> Result<Option<Member>>;
    async fn find_by_username(&self, username: &str) -> Result<Option<Member>>;
    /// Every member with a non-empty `discord_id`, regardless of
    /// status. Used by the Discord reconcile sweep so we can catch
    /// drift on Active / Honorary / Expired / Suspended members in
    /// one pass.
    async fn list_with_discord_id(&self) -> Result<Vec<Member>>;
    async fn update(&self, id: Uuid, update: UpdateMemberRequest) -> Result<Member>;
    async fn set_admin(&self, id: Uuid, is_admin: bool) -> Result<Member>;
    async fn mark_email_verified(&self, id: Uuid) -> Result<()>;
    async fn update_password_hash(&self, id: Uuid, password_hash: &str) -> Result<()>;
    /// Set or clear the member's Discord snowflake ID. `None` clears it.
    /// Validation is the caller's responsibility (see
    /// `integrations::discord::is_valid_snowflake`).
    async fn update_discord_id(&self, id: Uuid, discord_id: Option<&str>) -> Result<()>;
    /// Filtered, sorted, paginated lookup. Used by the admin members
    /// page; replaces the previous "list 1000 then filter in Rust"
    /// shape (which silently dropped rows past 1000 and used
    /// `format!("{:?}", status)` as a sort key tied to the Debug
    /// derive). Returns `(rows, total_match_count)` so the caller can
    /// compute total pages without a second round trip.
    async fn search(&self, query: MemberQuery) -> Result<(Vec<Member>, i64)>;
    /// Set the member's `dues_paid_until`, revive Expired→Active in
    /// the same UPDATE, and clear the dues-reminder flag so the next
    /// dues cycle can re-fire a reminder. Suspended/Honorary/Pending
    /// statuses are left untouched. This is the single source of
    /// truth for "a payment was just recorded" — every membership
    /// payment path goes through here.
    async fn set_dues_paid_until_with_revival(
        &self,
        id: Uuid,
        new_dues_paid_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<()>;
    /// Inverse of `set_dues_paid_until_with_revival`: backdate
    /// `dues_paid_until` to yesterday and flip Active→Expired in the
    /// same UPDATE. Pending/Suspended/Honorary are left alone (same
    /// asymmetric carve-outs as revival). Used by the admin "expire
    /// now" action; the billing runner would do this on its next tick
    /// anyway, but admins reasonably expect the change to be live
    /// immediately.
    async fn expire_dues_now(&self, id: Uuid) -> Result<()>;
    /// Stamp `dues_reminder_sent_at = CURRENT_TIMESTAMP`. Called from
    /// the dues-reminder runner once the email has gone out, so the
    /// next sweep won't re-send for this dues cycle. Cleared on
    /// payment via `set_dues_paid_until_with_revival`.
    async fn set_dues_reminder_sent(&self, id: Uuid) -> Result<()>;
    /// Update billing mode and subscription id atomically. Pass
    /// `Some(&id)` to set the Stripe subscription id, `None` to clear
    /// it (the right move when leaving `StripeSubscription`). Used by
    /// the auto-renew lifecycle in `BillingService` and by the
    /// Stripe webhook handler when a subscription gets cancelled
    /// out-of-band.
    async fn set_billing_mode(
        &self,
        id: Uuid,
        mode: BillingMode,
        stripe_subscription_id: Option<&str>,
    ) -> Result<()>;
    /// Persist the Stripe customer id for a member. Customer ids are
    /// created lazily on first charge / SetupIntent so this gets
    /// called exactly once per member's lifetime.
    async fn set_stripe_customer_id(&self, id: Uuid, customer_id: &str) -> Result<()>;
    /// Reverse of `stripe_customer_id` — given the Stripe-side id,
    /// find the Coterie member. The webhook handlers use this to
    /// route Stripe events back onto the right row.
    async fn find_by_stripe_customer_id(&self, customer_id: &str) -> Result<Option<Member>>;
    /// Count of members currently in a given billing mode. Drives
    /// the admin "Stripe-sub members remaining" badge.
    async fn count_by_billing_mode(&self, mode: BillingMode) -> Result<i64>;
    /// Member ids in a given billing mode. Used by the bulk-migrate
    /// job that flips every `stripe_subscription` member to
    /// `coterie_managed`.
    async fn list_ids_by_billing_mode(&self, mode: BillingMode) -> Result<Vec<Uuid>>;
}

// Database row struct that matches SQLite schema
#[derive(FromRow)]
struct MemberRow {
    id: String,
    email: String,
    username: String,
    full_name: String,
    status: String,
    membership_type_id: String,
    joined_at: NaiveDateTime,
    expires_at: Option<NaiveDateTime>,
    dues_paid_until: Option<NaiveDateTime>,
    bypass_dues: i32,
    is_admin: i32,
    notes: Option<String>,
    stripe_customer_id: Option<String>,
    stripe_subscription_id: Option<String>,
    billing_mode: String,
    email_verified_at: Option<NaiveDateTime>,
    dues_reminder_sent_at: Option<NaiveDateTime>,
    discord_id: Option<String>,
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
        let membership_type_id = Uuid::parse_str(&row.membership_type_id)
            .map_err(|e| AppError::Internal(e.to_string()))?;

        let billing_mode = BillingMode::from_str(&row.billing_mode)
            .unwrap_or(BillingMode::Manual);

        Ok(Member {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Internal(e.to_string()))?,
            email: row.email,
            username: row.username,
            full_name: row.full_name,
            status: Self::parse_member_status(&row.status)?,
            membership_type_id,
            joined_at: DateTime::from_naive_utc_and_offset(row.joined_at, Utc),
            expires_at: row.expires_at.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            dues_paid_until: row.dues_paid_until.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            bypass_dues: row.bypass_dues != 0,
            is_admin: row.is_admin != 0,
            notes: row.notes,
            stripe_customer_id: row.stripe_customer_id,
            stripe_subscription_id: row.stripe_subscription_id,
            billing_mode,
            email_verified_at: row.email_verified_at.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            dues_reminder_sent_at: row.dues_reminder_sent_at.map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            discord_id: row.discord_id,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }

    fn parse_member_status(s: &str) -> Result<MemberStatus> {
        MemberStatus::from_str(s)
            .ok_or_else(|| AppError::Internal(format!("Invalid member status: {}", s)))
    }

    /// Resolve a `CreateMemberRequest`'s membership_type_id, defaulting
    /// to the first `is_active` row in `membership_types` (sort_order
    /// ASC, name ASC) when the caller didn't provide one. Errors if no
    /// active type exists — an org with no active types can't accept
    /// signups, and silently picking an inactive type would mask the
    /// misconfiguration.
    async fn resolve_membership_type_id(&self, supplied: Option<Uuid>) -> Result<Uuid> {
        if let Some(id) = supplied {
            return Ok(id);
        }
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM membership_types \
             WHERE is_active = 1 \
             ORDER BY sort_order ASC, name ASC \
             LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some((id_str,)) => Uuid::parse_str(&id_str)
                .map_err(|e| AppError::Internal(e.to_string())),
            None => Err(AppError::BadRequest(
                "No active membership types configured — admin must create one before \
                 members can be added."
                    .to_string(),
            )),
        }
    }
}

#[async_trait]
impl MemberRepository for SqliteMemberRepository {
    async fn create(&self, request: CreateMemberRequest) -> Result<Member> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let status = MemberStatus::Pending;
        let membership_type_id = self.resolve_membership_type_id(request.membership_type_id).await?;

        // Hash the password with argon2
        use argon2::{Argon2, PasswordHasher};
        use argon2::password_hash::{SaltString, rand_core::OsRng};

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        let password_hash = argon2
            .hash_password(request.password.as_bytes(), &salt)
            .map_err(|e| AppError::Internal(e.to_string()))?
            .to_string();

        let status_str = status.as_str();
        let id_str = id.to_string();
        let mt_id_str = membership_type_id.to_string();
        let now_naive = now.naive_utc();

        sqlx::query(
            r#"
            INSERT INTO members (
                id, email, username, full_name, password_hash,
                status, membership_type_id, joined_at, bypass_dues,
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
        .bind(&mt_id_str)
        .bind(now_naive)
        .bind(0i32)  // bypass_dues as integer (0 = false)
        .bind(now_naive)
        .bind(now_naive)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Internal("Failed to retrieve created member".to_string())
        })
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Member>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT id, email, username, full_name, status, membership_type_id,
                   joined_at, expires_at, dues_paid_until, bypass_dues, is_admin, notes,
                   stripe_customer_id, stripe_subscription_id, billing_mode, email_verified_at,
                   dues_reminder_sent_at, discord_id, created_at, updated_at
            FROM members
            WHERE id = ?
            "#
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_member(r)?)),
            None => Ok(None)
        }
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<Member>> {
        let row = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT id, email, username, full_name, status, membership_type_id,
                   joined_at, expires_at, dues_paid_until, bypass_dues, is_admin, notes,
                   stripe_customer_id, stripe_subscription_id, billing_mode, email_verified_at,
                   dues_reminder_sent_at, discord_id, created_at, updated_at
            FROM members
            WHERE email = ?
            "#
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_member(r)?)),
            None => Ok(None)
        }
    }

    async fn find_by_username(&self, username: &str) -> Result<Option<Member>> {
        let row = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT id, email, username, full_name, status, membership_type_id,
                   joined_at, expires_at, dues_paid_until, bypass_dues, is_admin, notes,
                   stripe_customer_id, stripe_subscription_id, billing_mode, email_verified_at,
                   dues_reminder_sent_at, discord_id, created_at, updated_at
            FROM members
            WHERE username = ?
            "#
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_member(r)?)),
            None => Ok(None)
        }
    }

    async fn list_with_discord_id(&self) -> Result<Vec<Member>> {
        let rows = sqlx::query_as::<_, MemberRow>(
            r#"
            SELECT id, email, username, full_name, status, membership_type_id,
                   joined_at, expires_at, dues_paid_until, bypass_dues, is_admin, notes,
                   stripe_customer_id, stripe_subscription_id, billing_mode, email_verified_at,
                   dues_reminder_sent_at, discord_id, created_at, updated_at
            FROM members
            WHERE discord_id IS NOT NULL AND discord_id != ''
            ORDER BY status, joined_at
            "#
        )
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(Self::row_to_member)
            .collect()
    }

    async fn update(&self, id: Uuid, update: UpdateMemberRequest) -> Result<Member> {
        let existing = self.find_by_id(id).await?
            .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;

        let now = Utc::now();

        let status_str = update.status.as_ref().unwrap_or(&existing.status).as_str();
        let membership_type_id = update.membership_type_id.unwrap_or(existing.membership_type_id);
        let mt_id_str = membership_type_id.to_string();

        let id_str = id.to_string();
        let now_naive = now.naive_utc();
        let expires_at_naive = update.expires_at.map(|dt| dt.naive_utc());
        let bypass_dues_int = update.bypass_dues.map(|b| if b { 1i32 } else { 0i32 });

        sqlx::query(
            r#"
            UPDATE members
            SET full_name = COALESCE(?, full_name),
                status = ?,
                membership_type_id = ?,
                expires_at = COALESCE(?, expires_at),
                bypass_dues = COALESCE(?, bypass_dues),
                notes = COALESCE(?, notes),
                updated_at = ?
            WHERE id = ?
            "#
        )
        .bind(&update.full_name)
        .bind(status_str)
        .bind(&mt_id_str)
        .bind(expires_at_naive)
        .bind(bypass_dues_int)
        .bind(&update.notes)
        .bind(now_naive)
        .bind(&id_str)
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::Internal("Failed to retrieve updated member".to_string())
        })
    }

    async fn set_admin(&self, id: Uuid, is_admin: bool) -> Result<Member> {
        let id_str = id.to_string();
        let now_naive = Utc::now().naive_utc();
        let flag = if is_admin { 1i32 } else { 0i32 };

        sqlx::query("UPDATE members SET is_admin = ?, updated_at = ? WHERE id = ?")
            .bind(flag)
            .bind(now_naive)
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;

        self.find_by_id(id).await?.ok_or_else(|| {
            AppError::NotFound("Member not found".to_string())
        })
    }

    async fn mark_email_verified(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        let now_naive = Utc::now().naive_utc();
        sqlx::query(
            "UPDATE members SET email_verified_at = ?, updated_at = ? \
             WHERE id = ? AND email_verified_at IS NULL"
        )
            .bind(now_naive)
            .bind(now_naive)
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn update_password_hash(&self, id: Uuid, password_hash: &str) -> Result<()> {
        let id_str = id.to_string();
        let now_naive = Utc::now().naive_utc();
        sqlx::query(
            "UPDATE members SET password_hash = ?, updated_at = ? WHERE id = ?"
        )
            .bind(password_hash)
            .bind(now_naive)
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn update_discord_id(&self, id: Uuid, discord_id: Option<&str>) -> Result<()> {
        let id_str = id.to_string();
        let now_naive = Utc::now().naive_utc();
        sqlx::query(
            "UPDATE members SET discord_id = ?, updated_at = ? WHERE id = ?"
        )
            .bind(discord_id)
            .bind(now_naive)
            .bind(&id_str)
            .execute(&self.pool)
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }

    async fn set_dues_paid_until_with_revival(
        &self,
        id: Uuid,
        new_dues_paid_until: chrono::DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE members \
             SET dues_paid_until = ?, \
                 status = CASE WHEN status = 'Expired' THEN 'Active' ELSE status END, \
                 dues_reminder_sent_at = NULL, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(new_dues_paid_until)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn expire_dues_now(&self, id: Uuid) -> Result<()> {
        let yesterday = Utc::now() - chrono::Duration::days(1);
        sqlx::query(
            "UPDATE members \
             SET dues_paid_until = ?, \
                 status = CASE WHEN status = 'Active' THEN 'Expired' ELSE status END, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(yesterday)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn set_dues_reminder_sent(&self, id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE members \
             SET dues_reminder_sent_at = CURRENT_TIMESTAMP, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn set_billing_mode(
        &self,
        id: Uuid,
        mode: BillingMode,
        stripe_subscription_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE members \
             SET billing_mode = ?, \
                 stripe_subscription_id = ?, \
                 updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(mode.as_str())
        .bind(stripe_subscription_id)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn set_stripe_customer_id(&self, id: Uuid, customer_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE members \
             SET stripe_customer_id = ?, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(customer_id)
        .bind(id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }

    async fn find_by_stripe_customer_id(&self, customer_id: &str) -> Result<Option<Member>> {
        let row = sqlx::query_as::<_, MemberRow>(
            "SELECT id, email, username, full_name, status, membership_type_id, \
                    joined_at, expires_at, dues_paid_until, \
                    bypass_dues, is_admin, notes, stripe_customer_id, \
                    stripe_subscription_id, billing_mode, email_verified_at, \
                    dues_reminder_sent_at, discord_id, created_at, updated_at \
             FROM members WHERE stripe_customer_id = ?",
        )
        .bind(customer_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(AppError::Database)?;

        match row {
            Some(r) => Ok(Some(Self::row_to_member(r)?)),
            None => Ok(None),
        }
    }

    async fn count_by_billing_mode(&self, mode: BillingMode) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM members WHERE billing_mode = ?",
        )
        .bind(mode.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(count)
    }

    async fn list_ids_by_billing_mode(&self, mode: BillingMode) -> Result<Vec<Uuid>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM members WHERE billing_mode = ?",
        )
        .bind(mode.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;

        rows.into_iter()
            .map(|(id_str,)| {
                Uuid::parse_str(&id_str)
                    .map_err(|e| AppError::Internal(format!("Invalid uuid {}: {}", id_str, e)))
            })
            .collect()
    }

    async fn search(&self, query: MemberQuery) -> Result<(Vec<Member>, i64)> {
        // Build WHERE clause + bound params from the typed query.
        // Sort field/direction map to constant strings (no injection
        // risk); user-provided values (search, status, type) bind.
        let search_pat = query.search
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| format!("%{}%", s.to_lowercase()));
        let status_str = query.status.as_ref().map(|s| s.as_str().to_string());
        let mtype_id_str = query.membership_type_id.map(|id| id.to_string());

        let mut where_clauses: Vec<&str> = Vec::new();
        if search_pat.is_some() {
            where_clauses.push(
                "(LOWER(full_name) LIKE ? OR LOWER(email) LIKE ? OR LOWER(username) LIKE ?)",
            );
        }
        if status_str.is_some() {
            where_clauses.push("status = ?");
        }
        if mtype_id_str.is_some() {
            where_clauses.push("membership_type_id = ?");
        }
        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        // ORDER BY mapping. NULL dues_paid_until sorts to the bottom
        // regardless of direction (admins want "set" rows above "not
        // set" rows when sorting by that column).
        let order_dir = match query.order {
            SortOrder::Asc => "ASC",
            SortOrder::Desc => "DESC",
        };
        let order_sql = match query.sort {
            MemberSortField::Name => format!("LOWER(full_name) {}", order_dir),
            MemberSortField::Status => format!("status {}", order_dir),
            MemberSortField::MembershipType => format!("membership_type_id {}", order_dir),
            MemberSortField::Joined => format!("joined_at {}", order_dir),
            MemberSortField::DuesPaidUntil => {
                format!("dues_paid_until IS NULL, dues_paid_until {}", order_dir)
            }
        };

        let select_sql = format!(
            "SELECT id, email, username, full_name, status, membership_type_id, \
                    joined_at, expires_at, dues_paid_until, bypass_dues, is_admin, notes, \
                    stripe_customer_id, stripe_subscription_id, billing_mode, email_verified_at, \
                    dues_reminder_sent_at, discord_id, created_at, updated_at \
             FROM members{} \
             ORDER BY {} \
             LIMIT ? OFFSET ?",
            where_sql, order_sql,
        );
        let count_sql = format!("SELECT COUNT(*) FROM members{}", where_sql);

        // Bind WHERE params first (used by both queries), then LIMIT/OFFSET.
        let mut rows_q = sqlx::query_as::<_, MemberRow>(&select_sql);
        let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
        if let Some(p) = &search_pat {
            rows_q = rows_q.bind(p).bind(p).bind(p);
            count_q = count_q.bind(p).bind(p).bind(p);
        }
        if let Some(s) = &status_str {
            rows_q = rows_q.bind(s);
            count_q = count_q.bind(s);
        }
        if let Some(t) = &mtype_id_str {
            rows_q = rows_q.bind(t);
            count_q = count_q.bind(t);
        }
        rows_q = rows_q.bind(query.limit).bind(query.offset);

        let rows = rows_q.fetch_all(&self.pool).await
            .map_err(AppError::Database)?;
        let total: i64 = count_q.fetch_one(&self.pool).await
            .map_err(AppError::Database)?;

        let members = rows.into_iter().map(Self::row_to_member).collect::<Result<Vec<_>>>()?;
        Ok((members, total))
    }
}