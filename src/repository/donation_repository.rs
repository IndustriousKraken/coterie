use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::DonationCampaign,
    error::{AppError, Result},
    repository::DonationCampaignRepository,
};

#[derive(FromRow)]
struct CampaignRow {
    id: String,
    name: String,
    slug: String,
    description: Option<String>,
    goal_cents: Option<i64>,
    is_active: i32,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

pub struct SqliteDonationCampaignRepository {
    pool: SqlitePool,
}

impl SqliteDonationCampaignRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_campaign(row: CampaignRow) -> Result<DonationCampaign> {
        Ok(DonationCampaign {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            name: row.name,
            slug: row.slug,
            description: row.description,
            goal_cents: row.goal_cents,
            is_active: row.is_active != 0,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }
}

#[async_trait]
impl DonationCampaignRepository for SqliteDonationCampaignRepository {
    async fn create(&self, campaign: DonationCampaign) -> Result<DonationCampaign> {
        let id_str = campaign.id.to_string();
        let is_active_int = if campaign.is_active { 1 } else { 0 };
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            INSERT INTO donation_campaigns (id, name, slug, description, goal_cents, is_active, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id_str)
        .bind(&campaign.name)
        .bind(&campaign.slug)
        .bind(&campaign.description)
        .bind(campaign.goal_cents)
        .bind(is_active_int)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(campaign.id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve created campaign".to_string())
        })
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<DonationCampaign>> {
        let row = sqlx::query_as::<_, CampaignRow>(
            "SELECT id, name, slug, description, goal_cents, is_active, created_at, updated_at FROM donation_campaigns WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_campaign(r)?)),
            None => Ok(None),
        }
    }

    async fn find_by_slug(&self, slug: &str) -> Result<Option<DonationCampaign>> {
        let row = sqlx::query_as::<_, CampaignRow>(
            "SELECT id, name, slug, description, goal_cents, is_active, created_at, updated_at FROM donation_campaigns WHERE slug = ?",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_campaign(r)?)),
            None => Ok(None),
        }
    }

    async fn list_active(&self) -> Result<Vec<DonationCampaign>> {
        let rows = sqlx::query_as::<_, CampaignRow>(
            "SELECT id, name, slug, description, goal_cents, is_active, created_at, updated_at FROM donation_campaigns WHERE is_active = 1 ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter().map(Self::row_to_campaign).collect()
    }

    async fn get_total_donated(&self, campaign_id: Uuid) -> Result<i64> {
        let total: Option<i64> = sqlx::query_scalar(
            r#"
            SELECT COALESCE(SUM(p.amount_cents), 0)
            FROM payments p
            WHERE p.payment_type = 'donation'
              AND p.description LIKE '%' || ? || '%'
              AND p.status = 'Completed'
            "#,
        )
        .bind(campaign_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(total.unwrap_or(0))
    }
}
