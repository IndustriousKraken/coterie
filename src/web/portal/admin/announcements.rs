use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Multipart, Path, Query, State},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AnnouncementBasicTypeService,
    },
    auth::CsrfService,
    config::Settings,
    integrations::public_site::{self as public_site_kind, PublicSiteNotifier},
    repository::AnnouncementRepository,
    service::announcement_admin_service::{
        AnnouncementAdminService, CreateAnnouncementInput, UpdateAnnouncementInput,
    },
    service::settings_service::SettingsService,
    web::portal::admin::partials,
    web::templates::{BaseContext, HtmlTemplate},
    web::uploads::save_uploaded_file,
};

/// Parse the form's `scheduled_publish_at` value (HTML `datetime-local`,
/// `YYYY-MM-DDTHH:MM` or with seconds) into an Option<DateTime<Utc>>.
/// The value is a **local wall-clock** in the org timezone; it is stored
/// naive-as-is in a `DateTime<Utc>` container (paired with a frozen zone
/// on the row), NOT converted to a real UTC instant here — the runner
/// derives the true instant from (wall-clock, zone) at compare time.
/// Empty input or unparseable input → None (we treat invalid as "not
/// scheduled" rather than rejecting the whole form for v1 — form-side
/// validation can be tightened later).
fn parse_scheduled_publish_at(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // datetime-local typically submits `YYYY-MM-DDTHH:MM`; some browsers
    // emit `YYYY-MM-DDTHH:MM:SS`. Try both.
    let parsed = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S"))
        .ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
        parsed,
        chrono::Utc,
    ))
}

/// Simple struct for type options in dropdowns
#[derive(Clone)]
pub struct TypeOption {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub color: Option<String>,
}

#[derive(Template)]
#[template(path = "admin/announcements.html")]
pub struct AdminAnnouncementsTemplate {
    pub base: BaseContext,
    pub announcements: Vec<AdminAnnouncementInfo>,
    pub total_announcements: i64,
    pub current_page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub search_query: String,
    pub type_filter: String,
    pub status_filter: String,
    pub sort_field: String,
    pub sort_order: String,
}

#[derive(Template)]
#[template(path = "admin/announcements_table.html")]
pub struct AdminAnnouncementsTableTemplate {
    pub announcements: Vec<AdminAnnouncementInfo>,
    pub total_announcements: i64,
    pub current_page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub search_query: String,
    pub type_filter: String,
    pub status_filter: String,
    pub sort_field: String,
    pub sort_order: String,
}

pub struct AdminAnnouncementInfo {
    pub id: String,
    pub title: String,
    pub announcement_type: String,
    pub is_public: bool,
    pub featured: bool,
    pub published_at: Option<String>,
    pub is_published: bool,
    pub created_at: String,
    pub content_preview: String,
    pub image_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdminAnnouncementsQuery {
    pub q: Option<String>,
    #[serde(rename = "type")]
    pub announcement_type: Option<String>,
    pub status: Option<String>,
    pub page: Option<i64>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

pub async fn admin_announcements_page(
    State(announcement_repo): State<Arc<dyn AnnouncementRepository>>,
    State(csrf_service): State<Arc<CsrfService>>,
    headers: axum::http::HeaderMap,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<AdminAnnouncementsQuery>,
) -> impl IntoResponse {
    let is_htmx = headers.get("HX-Request").is_some();

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let page = query.page.unwrap_or(1).max(1);
    let per_page: i64 = 20;

    let search_query = query.q.clone().unwrap_or_default().to_lowercase();
    let type_filter = query.announcement_type.clone().unwrap_or_default();
    let status_filter = query.status.clone().unwrap_or_default();
    let sort_field = query
        .sort
        .clone()
        .unwrap_or_else(|| "created_at".to_string());
    let sort_order = query.order.clone().unwrap_or_else(|| "desc".to_string());

    let all_announcements = announcement_repo.list(1000, 0).await.unwrap_or_default();

    let mut filtered_announcements: Vec<_> = all_announcements
        .into_iter()
        .filter(|a| {
            if !search_query.is_empty() {
                let matches = a.title.to_lowercase().contains(&search_query)
                    || a.content.to_lowercase().contains(&search_query);
                if !matches {
                    return false;
                }
            }
            if !type_filter.is_empty() && format!("{:?}", a.announcement_type) != type_filter {
                return false;
            }
            if !status_filter.is_empty() {
                let is_published = a.published_at.is_some();
                match status_filter.as_str() {
                    "published" => {
                        if !is_published {
                            return false;
                        }
                    }
                    "draft" => {
                        if is_published {
                            return false;
                        }
                    }
                    "featured" => {
                        if !a.featured {
                            return false;
                        }
                    }
                    "public" => {
                        if !a.is_public {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
            true
        })
        .collect();

    match sort_field.as_str() {
        "title" => {
            filtered_announcements.sort_by(|a, b| {
                if sort_order == "asc" {
                    a.title.to_lowercase().cmp(&b.title.to_lowercase())
                } else {
                    b.title.to_lowercase().cmp(&a.title.to_lowercase())
                }
            });
        }
        "type" => {
            filtered_announcements.sort_by(|a, b| {
                let a_type = format!("{:?}", a.announcement_type);
                let b_type = format!("{:?}", b.announcement_type);
                if sort_order == "asc" {
                    a_type.cmp(&b_type)
                } else {
                    b_type.cmp(&a_type)
                }
            });
        }
        "published_at" => {
            filtered_announcements.sort_by(|a, b| {
                if sort_order == "asc" {
                    a.published_at.cmp(&b.published_at)
                } else {
                    b.published_at.cmp(&a.published_at)
                }
            });
        }
        _ => {
            filtered_announcements.sort_by(|a, b| {
                if sort_order == "asc" {
                    a.created_at.cmp(&b.created_at)
                } else {
                    b.created_at.cmp(&a.created_at)
                }
            });
        }
    }

    let total_announcements = filtered_announcements.len() as i64;
    let total_pages = (total_announcements + per_page - 1) / per_page;
    let offset = ((page - 1) * per_page) as usize;
    let paginated_announcements: Vec<AdminAnnouncementInfo> = filtered_announcements
        .into_iter()
        .skip(offset)
        .take(per_page as usize)
        .map(|a| {
            // Char-boundary-safe truncation: `&a.content[..100]` would panic
            // when byte index 100 lands inside a multi-byte UTF-8 character.
            let preview = crate::util::string::truncate_chars(&a.content, 100);
            let content_preview = if preview.len() < a.content.len() {
                format!("{}...", preview)
            } else {
                a.content.clone()
            };
            AdminAnnouncementInfo {
                id: a.id.to_string(),
                title: a.title,
                announcement_type: format!("{:?}", a.announcement_type),
                is_public: a.is_public,
                featured: a.featured,
                published_at: a
                    .published_at
                    .map(|dt| dt.format("%b %d, %Y %H:%M").to_string()),
                is_published: a.published_at.is_some(),
                created_at: a.created_at.format("%b %d, %Y").to_string(),
                content_preview,
                image_url: a.image_url,
            }
        })
        .collect();

    let search_query_val = query.q.unwrap_or_default();
    let type_filter_val = query.announcement_type.unwrap_or_default();
    let status_filter_val = query.status.unwrap_or_default();

    if is_htmx {
        HtmlTemplate(AdminAnnouncementsTableTemplate {
            announcements: paginated_announcements,
            total_announcements,
            current_page: page,
            per_page,
            total_pages,
            search_query: search_query_val,
            type_filter: type_filter_val,
            status_filter: status_filter_val,
            sort_field,
            sort_order,
        })
        .into_response()
    } else {
        HtmlTemplate(AdminAnnouncementsTemplate {
            base,
            announcements: paginated_announcements,
            total_announcements,
            current_page: page,
            per_page,
            total_pages,
            search_query: search_query_val,
            type_filter: type_filter_val,
            status_filter: status_filter_val,
            sort_field,
            sort_order,
        })
        .into_response()
    }
}

#[derive(Template)]
#[template(path = "admin/announcement_detail.html")]
pub struct AdminAnnouncementDetailTemplate {
    pub base: BaseContext,
    pub announcement: AdminAnnouncementDetail,
    pub announcement_types: Vec<TypeOption>,
    /// Whether a companion public site is configured. Gates the resend
    /// control entirely — see `AdminEventDetailTemplate`.
    pub public_site_configured: bool,
}

pub struct AdminAnnouncementDetail {
    pub id: String,
    pub title: String,
    /// Raw Markdown source of truth — shown in the edit textarea.
    pub content: String,
    /// Server-rendered sanitized HTML preview of `content` (read-only). The
    /// textarea edits the raw Markdown; this shows how it will display.
    pub content_html: String,
    pub announcement_type: String,
    pub is_public: bool,
    pub featured: bool,
    pub image_url: Option<String>,
    pub published_at: Option<String>,
    pub is_published: bool,
    pub created_at: String,
    pub updated_at: String,
    /// Form-input value for the `datetime-local` field — empty string
    /// if not scheduled, else the `YYYY-MM-DDTHH:MM` org-local wall-clock.
    pub scheduled_publish_at_input: String,
    /// Human-friendly display for the sidebar — None if not scheduled.
    pub scheduled_publish_at_display: Option<String>,
}

pub async fn admin_announcement_detail_page(
    State(announcement_repo): State<Arc<dyn AnnouncementRepository>>,
    State(announcement_type_service): State<AnnouncementBasicTypeService>,
    State(csrf_service): State<Arc<CsrfService>>,
    State(public_site): State<Arc<PublicSiteNotifier>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(announcement_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&announcement_id) {
        Ok(id) => id,
        Err(_) => {
            return partials::admin_alert("error", "Invalid announcement ID", false).into_response()
        }
    };

    let announcement = match announcement_repo.find_by_id(id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return partials::admin_alert("error", "Announcement not found", false).into_response()
        }
        Err(_) => {
            return partials::admin_alert("error", "Error loading announcement", false)
                .into_response()
        }
    };

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let scheduled_publish_at_input = announcement
        .scheduled_publish_at
        .map(|dt| dt.format("%Y-%m-%dT%H:%M").to_string())
        .unwrap_or_default();
    // Render the stored wall-clock labeled with the org zone abbreviation
    // (e.g. "9:00 AM EDT"), not a mislabeled "UTC" — the value is a local
    // wall-clock, and its derived instant may be offset-hours from UTC.
    let scheduled_publish_at_display = announcement.scheduled_publish_at.map(|dt| {
        let abbr = announcement
            .scheduled_zone_abbr()
            .unwrap_or_else(|| "UTC".to_string());
        format!("{} {}", dt.format("%b %d, %Y %H:%M"), abbr)
    });

    let content_html = crate::util::markdown::render_markdown(&announcement.content);
    let detail = AdminAnnouncementDetail {
        id: announcement.id.to_string(),
        title: announcement.title,
        content: announcement.content,
        content_html,
        announcement_type: format!("{:?}", announcement.announcement_type),
        is_public: announcement.is_public,
        featured: announcement.featured,
        image_url: announcement.image_url,
        published_at: announcement
            .published_at
            .map(|dt| dt.format("%b %d, %Y %H:%M").to_string()),
        is_published: announcement.published_at.is_some(),
        created_at: announcement
            .created_at
            .format("%b %d, %Y %H:%M")
            .to_string(),
        updated_at: announcement
            .updated_at
            .format("%b %d, %Y %H:%M")
            .to_string(),
        scheduled_publish_at_input,
        scheduled_publish_at_display,
    };

    // Fetch active announcement types for the dropdown
    let announcement_types = announcement_type_service
        .0
        .list(false)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| TypeOption {
            id: t.id.to_string(),
            name: t.name,
            slug: t.slug,
            color: t.color,
        })
        .collect();

    HtmlTemplate(AdminAnnouncementDetailTemplate {
        base,
        announcement: detail,
        announcement_types,
        public_site_configured: public_site.is_configured().await,
    })
    .into_response()
}

#[derive(Template)]
#[template(path = "admin/announcement_new.html")]
pub struct AdminNewAnnouncementTemplate {
    pub base: BaseContext,
    pub announcement_types: Vec<TypeOption>,
}

pub async fn admin_new_announcement_page(
    State(announcement_type_service): State<AnnouncementBasicTypeService>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    // Fetch active announcement types for the dropdown
    let announcement_types = announcement_type_service
        .0
        .list(false)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| TypeOption {
            id: t.id.to_string(),
            name: t.name,
            slug: t.slug,
            color: t.color,
        })
        .collect();

    HtmlTemplate(AdminNewAnnouncementTemplate {
        base,
        announcement_types,
    })
    .into_response()
}

pub async fn admin_create_announcement(
    State(settings): State<Arc<Settings>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(announcement_admin_service): State<Arc<AnnouncementAdminService>>,
    Extension(current_user): Extension<CurrentUser>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    use crate::domain::AnnouncementType;

    // Parse multipart form
    let mut title = String::new();
    let mut content = String::new();
    let mut announcement_type_str = String::new();
    let mut is_public = false;
    let mut featured = false;
    let mut publish_now = false;
    let mut image_url: Option<String> = None;
    let mut scheduled_publish_at_str = String::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "csrf_token" => {
                let _ = field.text().await;
            }
            "title" => title = field.text().await.unwrap_or_default(),
            "content" => content = field.text().await.unwrap_or_default(),
            "announcement_type" => announcement_type_str = field.text().await.unwrap_or_default(),
            "is_public" => {
                is_public = true;
                let _ = field.text().await;
            }
            "featured" => {
                featured = true;
                let _ = field.text().await;
            }
            "publish_now" => {
                publish_now = true;
                let _ = field.text().await;
            }
            "scheduled_publish_at" => {
                scheduled_publish_at_str = field.text().await.unwrap_or_default();
            }
            "image" => {
                let filename = field.file_name().unwrap_or("").to_string();
                if !filename.is_empty() {
                    if let Ok(data) = field.bytes().await {
                        if !data.is_empty() {
                            match save_uploaded_file(
                                &settings.server.uploads_path(),
                                &filename,
                                &data,
                            )
                            .await
                            {
                                Ok(path) => image_url = Some(path),
                                Err(e) => {
                                    return partials::admin_alert(
                                        "error",
                                        &format!("Error uploading image: {}", e),
                                        false,
                                    )
                                    .into_response()
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let announcement_type = match announcement_type_str.as_str() {
        "News" => AnnouncementType::News,
        "Achievement" => AnnouncementType::Achievement,
        "Meeting" => AnnouncementType::Meeting,
        "CTFResult" => AnnouncementType::CTFResult,
        "General" => AnnouncementType::General,
        _ => AnnouncementType::General,
    };

    let scheduled_publish_at = parse_scheduled_publish_at(&scheduled_publish_at_str);
    // Freeze the schedule's zone from the current org setting. The naive
    // form input is stored as-is (no conversion); the zone is what lets
    // the runner derive the correct publish instant. Mirrors event create.
    let scheduled_publish_timezone = settings_service.org_timezone().await.name().to_string();

    let input = CreateAnnouncementInput {
        title,
        content,
        announcement_type,
        announcement_type_id: None,
        is_public,
        featured,
        image_url,
        publish_now,
        scheduled_publish_at,
        scheduled_publish_timezone,
    };

    match announcement_admin_service
        .create(current_user.member.id, input)
        .await
    {
        Ok(created) => {
            axum::response::Redirect::to(&format!("/portal/admin/announcements/{}", created.id))
                .into_response()
        }
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error creating announcement: {}", e),
            false,
        )
        .into_response(),
    }
}

pub async fn admin_update_announcement(
    State(settings): State<Arc<Settings>>,
    State(settings_service): State<Arc<SettingsService>>,
    State(announcement_repo): State<Arc<dyn AnnouncementRepository>>,
    State(announcement_admin_service): State<Arc<AnnouncementAdminService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(announcement_id): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    use crate::domain::AnnouncementType;

    let id = match uuid::Uuid::parse_str(&announcement_id) {
        Ok(id) => id,
        Err(_) => {
            return partials::admin_alert("error", "Invalid announcement ID", false).into_response()
        }
    };

    let existing = match announcement_repo.find_by_id(id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return partials::admin_alert("error", "Announcement not found", false).into_response()
        }
        Err(_) => {
            return partials::admin_alert("error", "Error loading announcement", false)
                .into_response()
        }
    };

    // Parse multipart form
    let mut title = String::new();
    let mut content = String::new();
    let mut announcement_type_str = String::new();
    let mut is_public = false;
    let mut featured = false;
    let mut new_image_url: Option<String> = None;
    let mut remove_image = false;
    let mut scheduled_publish_at_str = String::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "csrf_token" => {
                let _ = field.text().await;
            }
            "title" => title = field.text().await.unwrap_or_default(),
            "content" => content = field.text().await.unwrap_or_default(),
            "announcement_type" => announcement_type_str = field.text().await.unwrap_or_default(),
            "is_public" => {
                is_public = true;
                let _ = field.text().await;
            }
            "featured" => {
                featured = true;
                let _ = field.text().await;
            }
            "remove_image" => {
                remove_image = true;
                let _ = field.text().await;
            }
            "scheduled_publish_at" => {
                scheduled_publish_at_str = field.text().await.unwrap_or_default();
            }
            "image" => {
                let filename = field.file_name().unwrap_or("").to_string();
                if !filename.is_empty() {
                    if let Ok(data) = field.bytes().await {
                        if !data.is_empty() {
                            match save_uploaded_file(
                                &settings.server.uploads_path(),
                                &filename,
                                &data,
                            )
                            .await
                            {
                                Ok(path) => new_image_url = Some(path),
                                Err(e) => {
                                    return partials::admin_alert(
                                        "error",
                                        &format!("Error uploading image: {}", e),
                                        false,
                                    )
                                    .into_response()
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let announcement_type = match announcement_type_str.as_str() {
        "News" => AnnouncementType::News,
        "Achievement" => AnnouncementType::Achievement,
        "Meeting" => AnnouncementType::Meeting,
        "CTFResult" => AnnouncementType::CTFResult,
        "General" => AnnouncementType::General,
        _ => AnnouncementType::General,
    };

    // Determine final image_url: new upload > remove > keep existing.
    // Capture the old URL so we can delete it from disk after save.
    let old_image = existing.image_url.clone();
    let image_url = if new_image_url.is_some() {
        new_image_url
    } else if remove_image {
        None
    } else {
        old_image.clone()
    };
    let image_to_delete = if image_url != old_image {
        old_image
    } else {
        None
    };

    let scheduled_publish_at = parse_scheduled_publish_at(&scheduled_publish_at_str);
    // Re-freeze the schedule's zone from the current org setting on each
    // submission (the edit form re-submits the schedule wall-clock too).
    let scheduled_publish_timezone = settings_service.org_timezone().await.name().to_string();

    let input = UpdateAnnouncementInput {
        title,
        content,
        announcement_type,
        announcement_type_id: existing.announcement_type_id,
        is_public,
        featured,
        image_url,
        scheduled_publish_at,
        scheduled_publish_timezone,
    };

    match announcement_admin_service
        .update(current_user.member.id, id, input)
        .await
    {
        Ok((_, public_site)) => {
            crate::web::uploads::delete_if_upload(
                &settings.server.uploads_path(),
                image_to_delete.as_deref(),
            )
            .await;
            // When the edit withdrew the announcement from the public
            // API, the admin is told plainly whether the public site
            // kept up, and where to retry when it did not.
            match public_site.admin_note() {
                Some(note) => partials::admin_alert(
                    if public_site.is_failed() {
                        "warning"
                    } else {
                        "success"
                    },
                    &format!("Announcement updated successfully. {}", note),
                    false,
                )
                .into_response(),
                None => axum::response::Html(r#"<div class="px-4 py-3 bg-green-100 text-green-800 rounded-md text-sm">Announcement updated successfully</div>"#.to_string()).into_response(),
            }
        }
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error updating announcement: {}", e),
            false,
        )
        .into_response(),
    }
}

pub async fn admin_delete_announcement(
    State(settings): State<Arc<Settings>>,
    State(announcement_repo): State<Arc<dyn AnnouncementRepository>>,
    State(announcement_admin_service): State<Arc<AnnouncementAdminService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(announcement_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&announcement_id) {
        Ok(id) => id,
        Err(_) => {
            return partials::admin_alert("error", "Invalid announcement ID", false).into_response()
        }
    };

    let image_to_delete = announcement_repo
        .find_by_id(id)
        .await
        .ok()
        .flatten()
        .and_then(|a| a.image_url);

    match announcement_admin_service
        .delete(current_user.member.id, id)
        .await
    {
        Ok(public_site) => {
            crate::web::uploads::delete_if_upload(
                &settings.server.uploads_path(),
                image_to_delete.as_deref(),
            )
            .await;
            partials::redirect_or_warn(
                "/portal/admin/announcements",
                &public_site,
                "Announcement deleted.",
            )
        }
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error deleting announcement: {}", e),
            false,
        )
        .into_response(),
    }
}

pub async fn admin_publish_announcement(
    State(announcement_admin_service): State<Arc<AnnouncementAdminService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(announcement_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&announcement_id) {
        Ok(id) => id,
        Err(_) => {
            return partials::admin_alert("error", "Invalid announcement ID", false).into_response()
        }
    };

    match announcement_admin_service
        .publish(current_user.member.id, id)
        .await
    {
        Ok(_) => axum::response::Redirect::to(&format!("/portal/admin/announcements/{}", id))
            .into_response(),
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error publishing announcement: {}", e),
            false,
        )
        .into_response(),
    }
}

pub async fn admin_unpublish_announcement(
    State(announcement_admin_service): State<Arc<AnnouncementAdminService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(announcement_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&announcement_id) {
        Ok(id) => id,
        Err(_) => {
            return partials::admin_alert("error", "Invalid announcement ID", false).into_response()
        }
    };

    match announcement_admin_service
        .unpublish(current_user.member.id, id)
        .await
    {
        // Unpublishing is the archetypal withdrawal: the admin gets a
        // plain answer about whether the public site is up to date, and
        // stays on a page that carries the resend control when it is not.
        Ok((_, public_site)) if public_site.is_failed() => partials::admin_alert(
            "warning",
            &format!(
                "Announcement unpublished. {}",
                public_site.admin_note().unwrap_or_default()
            ),
            false,
        )
        .into_response(),
        Ok(_) => axum::response::Redirect::to(&format!("/portal/admin/announcements/{}", id))
            .into_response(),
        Err(e) => partials::admin_alert(
            "error",
            &format!("Error unpublishing announcement: {}", e),
            false,
        )
        .into_response(),
    }
}

/// Resend this announcement's current state to the configured public
/// site. Mirrors the event resend control — same purpose, same
/// independence from the rest of the capability working.
pub async fn admin_resend_announcement_to_public_site(
    State(public_site): State<Arc<PublicSiteNotifier>>,
    Path(announcement_id): Path<String>,
) -> impl IntoResponse {
    let Ok(id) = uuid::Uuid::parse_str(&announcement_id) else {
        return partials::admin_alert("error", "Invalid announcement ID", false);
    };
    partials::resend_result(
        public_site
            .resend(public_site_kind::KIND_ANNOUNCEMENT, id)
            .await,
    )
}
