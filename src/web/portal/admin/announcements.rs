use askama::Template;
use axum::{
    extract::{State, Query, Path, Multipart},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    web::templates::{HtmlTemplate, UserInfo},
    web::uploads::save_uploaded_file,
};
use crate::web::portal::is_admin;

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
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
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
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<AdminAnnouncementsQuery>,
) -> impl IntoResponse {
    let is_htmx = headers.get("HX-Request").is_some();

    if !is_admin(&current_user.member) {
        return HtmlTemplate(AdminAnnouncementsTemplate {
            current_user: None,
            is_admin: false,
            csrf_token: String::new(),
            announcements: vec![],
            total_announcements: 0,
            current_page: 1,
            per_page: 20,
            total_pages: 0,
            search_query: String::new(),
            type_filter: String::new(),
            status_filter: String::new(),
            sort_field: "created_at".to_string(),
            sort_order: "desc".to_string(),
        }).into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    let page = query.page.unwrap_or(1).max(1);
    let per_page: i64 = 20;

    let search_query = query.q.clone().unwrap_or_default().to_lowercase();
    let type_filter = query.announcement_type.clone().unwrap_or_default();
    let status_filter = query.status.clone().unwrap_or_default();
    let sort_field = query.sort.clone().unwrap_or_else(|| "created_at".to_string());
    let sort_order = query.order.clone().unwrap_or_else(|| "desc".to_string());

    let all_announcements = state.service_context.announcement_repo
        .list(1000, 0)
        .await
        .unwrap_or_default();

    let mut filtered_announcements: Vec<_> = all_announcements.into_iter()
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
                    "published" => if !is_published { return false; },
                    "draft" => if is_published { return false; },
                    "featured" => if !a.featured { return false; },
                    "public" => if !a.is_public { return false; },
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
            let content_preview = if a.content.len() > 100 {
                format!("{}...", &a.content[..100])
            } else {
                a.content.clone()
            };
            AdminAnnouncementInfo {
                id: a.id.to_string(),
                title: a.title,
                announcement_type: format!("{:?}", a.announcement_type),
                is_public: a.is_public,
                featured: a.featured,
                published_at: a.published_at.map(|dt| dt.format("%b %d, %Y %H:%M").to_string()),
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
        }).into_response()
    } else {
        HtmlTemplate(AdminAnnouncementsTemplate {
            current_user: Some(user_info),
            is_admin: true,
            csrf_token,
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
        }).into_response()
    }
}

#[derive(Template)]
#[template(path = "admin/announcement_detail.html")]
pub struct AdminAnnouncementDetailTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub announcement: AdminAnnouncementDetail,
    pub announcement_types: Vec<TypeOption>,
}

pub struct AdminAnnouncementDetail {
    pub id: String,
    pub title: String,
    pub content: String,
    pub announcement_type: String,
    pub is_public: bool,
    pub featured: bool,
    pub image_url: Option<String>,
    pub published_at: Option<String>,
    pub is_published: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn admin_announcement_detail_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(announcement_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&announcement_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid announcement ID".to_string()).into_response(),
    };

    let announcement = match state.service_context.announcement_repo.find_by_id(id).await {
        Ok(Some(a)) => a,
        Ok(None) => return axum::response::Html("Announcement not found".to_string()).into_response(),
        Err(_) => return axum::response::Html("Error loading announcement".to_string()).into_response(),
    };

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    let detail = AdminAnnouncementDetail {
        id: announcement.id.to_string(),
        title: announcement.title,
        content: announcement.content,
        announcement_type: format!("{:?}", announcement.announcement_type),
        is_public: announcement.is_public,
        featured: announcement.featured,
        image_url: announcement.image_url,
        published_at: announcement.published_at.map(|dt| dt.format("%b %d, %Y %H:%M").to_string()),
        is_published: announcement.published_at.is_some(),
        created_at: announcement.created_at.format("%b %d, %Y %H:%M").to_string(),
        updated_at: announcement.updated_at.format("%b %d, %Y %H:%M").to_string(),
    };

    // Fetch active announcement types for the dropdown
    let announcement_types = state.service_context.announcement_type_service
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
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        announcement: detail,
        announcement_types,
    }).into_response()
}

#[derive(Template)]
#[template(path = "admin/announcement_new.html")]
pub struct AdminNewAnnouncementTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub announcement_types: Vec<TypeOption>,
}

pub async fn admin_new_announcement_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let user_info = UserInfo {
        id: current_user.member.id.to_string(),
        username: current_user.member.username.clone(),
        email: current_user.member.email.clone(),
    };

    let csrf_token = state.service_context.csrf_service
        .generate_token(&session_info.session_id)
        .await
        .unwrap_or_else(|_| "error".to_string());

    // Fetch active announcement types for the dropdown
    let announcement_types = state.service_context.announcement_type_service
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
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        announcement_types,
    }).into_response()
}

pub async fn admin_create_announcement(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    use crate::domain::{Announcement, AnnouncementType};

    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    // Parse multipart form
    let mut title = String::new();
    let mut content = String::new();
    let mut announcement_type_str = String::new();
    let mut is_public = false;
    let mut featured = false;
    let mut publish_now = false;
    let mut image_url: Option<String> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "csrf_token" => { let _ = field.text().await; }
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
            "image" => {
                let filename = field.file_name().unwrap_or("").to_string();
                if !filename.is_empty() {
                    if let Ok(data) = field.bytes().await {
                        if !data.is_empty() {
                            match save_uploaded_file(&state.settings.server.uploads_path(), &filename, &data).await {
                                Ok(path) => image_url = Some(path),
                                Err(e) => return axum::response::Html(format!("Error uploading image: {}", e)).into_response(),
                            }
                        }
                    }
                }
            }
            _ => { let _ = field.bytes().await; }
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

    let published_at = if publish_now {
        Some(chrono::Utc::now())
    } else {
        None
    };

    let announcement = Announcement {
        id: uuid::Uuid::new_v4(),
        title,
        content,
        announcement_type,
        announcement_type_id: None,
        is_public,
        featured,
        image_url,
        published_at,
        created_by: current_user.member.id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    match state.service_context.announcement_repo.create(announcement).await {
        Ok(created) => axum::response::Redirect::to(&format!("/portal/admin/announcements/{}", created.id)).into_response(),
        Err(e) => axum::response::Html(format!("Error creating announcement: {}", e)).into_response(),
    }
}

pub async fn admin_update_announcement(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(announcement_id): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    use crate::domain::{Announcement, AnnouncementType};

    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&announcement_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid announcement ID".to_string()).into_response(),
    };

    let existing = match state.service_context.announcement_repo.find_by_id(id).await {
        Ok(Some(a)) => a,
        Ok(None) => return axum::response::Html("Announcement not found".to_string()).into_response(),
        Err(_) => return axum::response::Html("Error loading announcement".to_string()).into_response(),
    };

    // Parse multipart form
    let mut title = String::new();
    let mut content = String::new();
    let mut announcement_type_str = String::new();
    let mut is_public = false;
    let mut featured = false;
    let mut new_image_url: Option<String> = None;
    let mut remove_image = false;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "csrf_token" => { let _ = field.text().await; }
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
            "image" => {
                let filename = field.file_name().unwrap_or("").to_string();
                if !filename.is_empty() {
                    if let Ok(data) = field.bytes().await {
                        if !data.is_empty() {
                            match save_uploaded_file(&state.settings.server.uploads_path(), &filename, &data).await {
                                Ok(path) => new_image_url = Some(path),
                                Err(e) => return axum::response::Html(format!(r#"<div class="px-4 py-3 bg-red-100 text-red-800 rounded-md text-sm">Error uploading image: {}</div>"#, e)).into_response(),
                            }
                        }
                    }
                }
            }
            _ => { let _ = field.bytes().await; }
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

    // Determine final image_url: new upload > remove > keep existing
    let image_url = if new_image_url.is_some() {
        new_image_url
    } else if remove_image {
        None
    } else {
        existing.image_url.clone()
    };

    let updated_announcement = Announcement {
        id,
        title,
        content,
        announcement_type,
        announcement_type_id: existing.announcement_type_id,
        is_public,
        featured,
        image_url,
        published_at: existing.published_at,
        created_by: existing.created_by,
        created_at: existing.created_at,
        updated_at: chrono::Utc::now(),
    };

    match state.service_context.announcement_repo.update(id, updated_announcement).await {
        Ok(_) => {
            axum::response::Html(r#"<div class="px-4 py-3 bg-green-100 text-green-800 rounded-md text-sm">Announcement updated successfully</div>"#.to_string()).into_response()
        }
        Err(e) => {
            axum::response::Html(format!(r#"<div class="px-4 py-3 bg-red-100 text-red-800 rounded-md text-sm">Error updating announcement: {}</div>"#, e)).into_response()
        }
    }
}

pub async fn admin_delete_announcement(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(announcement_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&announcement_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid announcement ID".to_string()).into_response(),
    };

    match state.service_context.announcement_repo.delete(id).await {
        Ok(_) => axum::response::Redirect::to("/portal/admin/announcements").into_response(),
        Err(e) => axum::response::Html(format!("Error deleting announcement: {}", e)).into_response(),
    }
}

pub async fn admin_publish_announcement(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(announcement_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&announcement_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid announcement ID".to_string()).into_response(),
    };

    let existing = match state.service_context.announcement_repo.find_by_id(id).await {
        Ok(Some(a)) => a,
        Ok(None) => return axum::response::Html("Announcement not found".to_string()).into_response(),
        Err(_) => return axum::response::Html("Error loading announcement".to_string()).into_response(),
    };

    let mut updated = existing;
    updated.published_at = Some(chrono::Utc::now());
    updated.updated_at = chrono::Utc::now();

    match state.service_context.announcement_repo.update(id, updated).await {
        Ok(_) => axum::response::Redirect::to(&format!("/portal/admin/announcements/{}", id)).into_response(),
        Err(e) => axum::response::Html(format!("Error publishing announcement: {}", e)).into_response(),
    }
}

pub async fn admin_unpublish_announcement(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Path(announcement_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin(&current_user.member) {
        return axum::response::Html("Access denied".to_string()).into_response();
    }

    let id = match uuid::Uuid::parse_str(&announcement_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("Invalid announcement ID".to_string()).into_response(),
    };

    let existing = match state.service_context.announcement_repo.find_by_id(id).await {
        Ok(Some(a)) => a,
        Ok(None) => return axum::response::Html("Announcement not found".to_string()).into_response(),
        Err(_) => return axum::response::Html("Error loading announcement".to_string()).into_response(),
    };

    let mut updated = existing;
    updated.published_at = None;
    updated.updated_at = chrono::Utc::now();

    match state.service_context.announcement_repo.update(id, updated).await {
        Ok(_) => axum::response::Redirect::to(&format!("/portal/admin/announcements/{}", id)).into_response(),
        Err(e) => axum::response::Html(format!("Error unpublishing announcement: {}", e)).into_response(),
    }
}
