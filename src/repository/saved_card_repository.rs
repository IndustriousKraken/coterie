use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::{
    domain::SavedCard,
    error::{AppError, Result},
    repository::SavedCardRepository,
};

#[derive(FromRow)]
struct SavedCardRow {
    id: String,
    member_id: String,
    stripe_payment_method_id: String,
    card_last_four: String,
    card_brand: String,
    exp_month: i32,
    exp_year: i32,
    is_default: i32,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
}

pub struct SqliteSavedCardRepository {
    pool: SqlitePool,
}

impl SqliteSavedCardRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn row_to_card(row: SavedCardRow) -> Result<SavedCard> {
        Ok(SavedCard {
            id: Uuid::parse_str(&row.id).map_err(|e| AppError::Database(e.to_string()))?,
            member_id: Uuid::parse_str(&row.member_id)
                .map_err(|e| AppError::Database(e.to_string()))?,
            stripe_payment_method_id: row.stripe_payment_method_id,
            card_last_four: row.card_last_four,
            card_brand: row.card_brand,
            exp_month: row.exp_month,
            exp_year: row.exp_year,
            is_default: row.is_default != 0,
            created_at: DateTime::from_naive_utc_and_offset(row.created_at, Utc),
            updated_at: DateTime::from_naive_utc_and_offset(row.updated_at, Utc),
        })
    }
}

#[async_trait]
impl SavedCardRepository for SqliteSavedCardRepository {
    async fn create(&self, card: SavedCard) -> Result<SavedCard> {
        let id_str = card.id.to_string();
        let member_id_str = card.member_id.to_string();
        let is_default_int = if card.is_default { 1 } else { 0 };
        let now = Utc::now().naive_utc();

        sqlx::query(
            r#"
            INSERT INTO payment_methods (
                id, member_id, stripe_payment_method_id, card_last_four,
                card_brand, exp_month, exp_year, is_default, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id_str)
        .bind(&member_id_str)
        .bind(&card.stripe_payment_method_id)
        .bind(&card.card_last_four)
        .bind(&card.card_brand)
        .bind(card.exp_month)
        .bind(card.exp_year)
        .bind(is_default_int)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        self.find_by_id(card.id).await?.ok_or_else(|| {
            AppError::Database("Failed to retrieve created card".to_string())
        })
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<SavedCard>> {
        let id_str = id.to_string();
        let row = sqlx::query_as::<_, SavedCardRow>(
            r#"
            SELECT id, member_id, stripe_payment_method_id, card_last_four,
                   card_brand, exp_month, exp_year, is_default, created_at, updated_at
            FROM payment_methods
            WHERE id = ?
            "#,
        )
        .bind(id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_card(r)?)),
            None => Ok(None),
        }
    }

    async fn find_by_member(&self, member_id: Uuid) -> Result<Vec<SavedCard>> {
        let member_id_str = member_id.to_string();
        let rows = sqlx::query_as::<_, SavedCardRow>(
            r#"
            SELECT id, member_id, stripe_payment_method_id, card_last_four,
                   card_brand, exp_month, exp_year, is_default, created_at, updated_at
            FROM payment_methods
            WHERE member_id = ?
            ORDER BY is_default DESC, created_at DESC
            "#,
        )
        .bind(member_id_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        rows.into_iter().map(Self::row_to_card).collect()
    }

    async fn find_default_for_member(&self, member_id: Uuid) -> Result<Option<SavedCard>> {
        let member_id_str = member_id.to_string();
        let row = sqlx::query_as::<_, SavedCardRow>(
            r#"
            SELECT id, member_id, stripe_payment_method_id, card_last_four,
                   card_brand, exp_month, exp_year, is_default, created_at, updated_at
            FROM payment_methods
            WHERE member_id = ? AND is_default = 1
            "#,
        )
        .bind(member_id_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_card(r)?)),
            None => Ok(None),
        }
    }

    async fn set_default(&self, member_id: Uuid, card_id: Uuid) -> Result<()> {
        let member_id_str = member_id.to_string();
        let card_id_str = card_id.to_string();
        let now = Utc::now().naive_utc();

        // Clear existing default
        sqlx::query(
            r#"
            UPDATE payment_methods
            SET is_default = 0, updated_at = ?
            WHERE member_id = ?
            "#,
        )
        .bind(now)
        .bind(&member_id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        // Set new default
        sqlx::query(
            r#"
            UPDATE payment_methods
            SET is_default = 1, updated_at = ?
            WHERE id = ? AND member_id = ?
            "#,
        )
        .bind(now)
        .bind(&card_id_str)
        .bind(&member_id_str)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::Database(e.to_string()))?;

        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        sqlx::query("DELETE FROM payment_methods WHERE id = ?")
            .bind(id_str)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}
