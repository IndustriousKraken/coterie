//! Public announcements surface. The full admin CRUD on announcements
//! lives in the portal (`web/portal/admin/announcements.rs`); the only
//! JSON endpoint on announcements is the count of members-only
//! published announcements, exposed to the public marketing site so it
//! can show "N members-only posts available — sign up" CTAs.

use std::sync::Arc;

use axum::{extract::State, Json};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    error::Result,
    repository::AnnouncementRepository,
};

#[derive(Serialize, ToSchema)]
pub struct PrivateAnnouncementCount {
    pub count: i64,
}

/// Returns the count of members-only published announcements.
/// This is a public endpoint to let visitors know there's exclusive content.
#[utoipa::path(
    get,
    path = "/public/announcements/private-count",
    tag = "public",
    responses(
        (status = 200, description = "Count of published members-only announcements",
            body = PrivateAnnouncementCount),
    ),
)]
pub async fn private_count(
    State(announcement_repo): State<Arc<dyn AnnouncementRepository>>,
) -> Result<Json<PrivateAnnouncementCount>> {
    let count = announcement_repo
        .count_private_published()
        .await?;

    Ok(Json(PrivateAnnouncementCount { count }))
}
