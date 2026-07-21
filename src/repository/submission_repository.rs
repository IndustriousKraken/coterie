use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::{Submission, SubmissionStatus, SubmissionVisibility},
    error::{AppError, Result},
};

#[async_trait]
pub trait SubmissionRepository: Send + Sync {
    async fn create(&self, submission: Submission) -> Result<Submission>;
    /// Reads used for authorization MUST return the full row (including
    /// `submitter_member_id`) so the caller can check ownership.
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Submission>>;
    async fn list_for_member(&self, member_id: Uuid) -> Result<Vec<Submission>>;
    /// The admin review queue: everything, newest first.
    async fn list_for_review(&self) -> Result<Vec<Submission>>;
    /// Count of a member's open (non-terminal) submissions — bounds the
    /// per-member create cap.
    async fn count_open_for_member(&self, member_id: Uuid) -> Result<i64>;
    async fn update(&self, submission: Submission) -> Result<Submission>;
}

#[derive(FromRow)]
struct SubmissionRow {
    id: String,
    submitter_member_id: String,
    title: String,
    abstract_text: String,
    visibility_requested: String,
    attachment_path: Option<String>,
    preferred_start: Option<NaiveDateTime>,
    timezone: String,
    duration_minutes: Option<i64>,
    status: String,
    reviewer_note: Option<String>,
    decided_by: Option<String>,
    event_id: Option<String>,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

pub struct SqliteSubmissionRepository {
    pool: SqlitePool,
}

impl SqliteSubmissionRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_submission(row: SubmissionRow) -> Result<Submission> {
        let parse_uuid =
            |s: &str| Uuid::parse_str(s).map_err(|e| AppError::Internal(e.to_string()));
        Ok(Submission {
            id: parse_uuid(&row.id)?,
            submitter_member_id: parse_uuid(&row.submitter_member_id)?,
            title: row.title,
            abstract_text: row.abstract_text,
            visibility_requested: SubmissionVisibility::from_wire(&row.visibility_requested),
            attachment_path: row.attachment_path,
            preferred_start: row
                .preferred_start
                .map(|dt| DateTime::from_naive_utc_and_offset(dt, Utc)),
            timezone: row.timezone,
            duration_minutes: row.duration_minutes.map(|d| d as i32),
            status: SubmissionStatus::from_wire(&row.status).ok_or_else(|| {
                AppError::Internal(format!("Invalid submission status: {}", row.status))
            })?,
            reviewer_note: row.reviewer_note,
            decided_by: row.decided_by.as_deref().map(parse_uuid).transpose()?,
            event_id: row.event_id.as_deref().map(parse_uuid).transpose()?,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }

    const SELECT_COLS: &'static str = "id, submitter_member_id, title, abstract_text, \
         visibility_requested, attachment_path, preferred_start, timezone, duration_minutes, \
         status, reviewer_note, decided_by, event_id, created_at, updated_at";
}

#[async_trait]
impl SubmissionRepository for SqliteSubmissionRepository {
    async fn create(&self, submission: Submission) -> Result<Submission> {
        let preferred_start = submission.preferred_start.map(|dt| dt.naive_utc());
        sqlx::query(
            r#"
            INSERT INTO submissions (
                id, submitter_member_id, title, abstract_text, visibility_requested,
                attachment_path, preferred_start, timezone, duration_minutes, status,
                reviewer_note, decided_by, event_id, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(submission.id.to_string())
        .bind(submission.submitter_member_id.to_string())
        .bind(&submission.title)
        .bind(&submission.abstract_text)
        .bind(submission.visibility_requested.as_wire())
        .bind(&submission.attachment_path)
        .bind(preferred_start)
        .bind(&submission.timezone)
        .bind(submission.duration_minutes)
        .bind(submission.status.as_wire())
        .bind(&submission.reviewer_note)
        .bind(submission.decided_by.map(|u| u.to_string()))
        .bind(submission.event_id.map(|u| u.to_string()))
        .bind(submission.created_at.naive_utc())
        .bind(submission.updated_at.naive_utc())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(submission.id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve created submission".to_string()))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Submission>> {
        let query = format!("SELECT {} FROM submissions WHERE id = ?", Self::SELECT_COLS);
        let row = sqlx::query_as::<_, SubmissionRow>(&query)
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;
        row.map(Self::row_to_submission).transpose()
    }

    async fn list_for_member(&self, member_id: Uuid) -> Result<Vec<Submission>> {
        let query = format!(
            "SELECT {} FROM submissions WHERE submitter_member_id = ? ORDER BY created_at DESC",
            Self::SELECT_COLS
        );
        let rows = sqlx::query_as::<_, SubmissionRow>(&query)
            .bind(member_id.to_string())
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;
        rows.into_iter().map(Self::row_to_submission).collect()
    }

    async fn list_for_review(&self) -> Result<Vec<Submission>> {
        let query = format!(
            "SELECT {} FROM submissions ORDER BY created_at DESC",
            Self::SELECT_COLS
        );
        let rows = sqlx::query_as::<_, SubmissionRow>(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;
        rows.into_iter().map(Self::row_to_submission).collect()
    }

    async fn count_open_for_member(&self, member_id: Uuid) -> Result<i64> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM submissions \
             WHERE submitter_member_id = ? AND status IN ('submitted', 'under_review')",
        )
        .bind(member_id.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(AppError::Database)?;
        Ok(count.0)
    }

    async fn update(&self, submission: Submission) -> Result<Submission> {
        let preferred_start = submission.preferred_start.map(|dt| dt.naive_utc());
        sqlx::query(
            r#"
            UPDATE submissions SET
                title = ?, abstract_text = ?, visibility_requested = ?, attachment_path = ?,
                preferred_start = ?, timezone = ?, duration_minutes = ?, status = ?,
                reviewer_note = ?, decided_by = ?, event_id = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&submission.title)
        .bind(&submission.abstract_text)
        .bind(submission.visibility_requested.as_wire())
        .bind(&submission.attachment_path)
        .bind(preferred_start)
        .bind(&submission.timezone)
        .bind(submission.duration_minutes)
        .bind(submission.status.as_wire())
        .bind(&submission.reviewer_note)
        .bind(submission.decided_by.map(|u| u.to_string()))
        .bind(submission.event_id.map(|u| u.to_string()))
        .bind(Utc::now().naive_utc())
        .bind(submission.id.to_string())
        .execute(&self.pool)
        .await
        .map_err(AppError::Database)?;

        self.find_by_id(submission.id)
            .await?
            .ok_or_else(|| AppError::Internal("Failed to retrieve updated submission".to_string()))
    }
}
