use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    repository::AnnouncementRepository,
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "portal/announcements.html")]
pub struct AnnouncementsTemplate {
    pub base: BaseContext,
}

pub async fn announcements_page(
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let template = AnnouncementsTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session).await,
    };

    HtmlTemplate(template)
}

#[derive(Debug, Deserialize)]
pub struct AnnouncementsListQuery {
    pub announcement_type: Option<String>,
    // HTML checkbox serializes as `show_all=on` when checked, absent when not — a
    // bare String parses both; presence (not value) means "show all".
    pub show_all: Option<String>,
}

pub async fn announcements_list_api(
    State(announcement_repo): State<Arc<dyn AnnouncementRepository>>,
    Extension(_current_user): Extension<CurrentUser>,
    Query(query): Query<AnnouncementsListQuery>,
) -> impl IntoResponse {
    // Get all published announcements (both public and private - members can see all)
    let limit = if query.show_all.is_some() { 100 } else { 20 };
    let announcements = announcement_repo
        .list_recent(limit)
        .await
        .unwrap_or_default();

    // Filter by type if specified
    let filtered_announcements: Vec<_> = announcements
        .into_iter()
        .filter(|a| {
            if let Some(ref announcement_type) = query.announcement_type {
                if !announcement_type.is_empty()
                    && format!("{:?}", a.announcement_type) != *announcement_type
                {
                    return false;
                }
            }
            true
        })
        .collect();

    if filtered_announcements.is_empty() {
        return axum::response::Html(
            r#"<div class="bg-white rounded-lg shadow-sm p-6 text-center text-gray-500">
                No announcements found
            </div>"#
                .to_string(),
        );
    }

    let mut html = String::new();
    html.push_str(r#"<div class="space-y-4">"#);

    for announcement in filtered_announcements {
        let type_badge_color = match format!("{:?}", announcement.announcement_type).as_str() {
            "News" => "bg-blue-100 text-blue-800",
            "Achievement" => "bg-yellow-100 text-yellow-800",
            "Meeting" => "bg-purple-100 text-purple-800",
            "CTFResult" => "bg-red-100 text-red-800",
            "General" => "bg-gray-100 text-gray-800",
            _ => "bg-gray-100 text-gray-800",
        };

        let visibility_badge = if announcement.is_public {
            ""
        } else {
            r#"<span class="px-2 py-1 text-xs font-medium rounded bg-indigo-100 text-indigo-800">Members Only</span>"#
        };

        let featured_badge = if announcement.featured {
            r#"<span class="px-2 py-1 text-xs font-medium rounded bg-amber-100 text-amber-800">Featured</span>"#
        } else {
            ""
        };

        let image_html = announcement.image_url.as_ref().map(|url| {
            format!(r#"<div class="bg-gray-100 rounded-t-lg -mt-6 -mx-6 mb-4 overflow-hidden" style="width: calc(100% + 3rem);"><img src="/{}" alt="" class="w-full h-40 object-contain"></div>"#, crate::web::escape_html(url))
        }).unwrap_or_default();

        let published_date = announcement
            .published_at
            .map(|dt| dt.format("%B %d, %Y").to_string())
            .unwrap_or_default();

        html.push_str(&format!(
            r#"<div class="bg-white rounded-lg shadow-sm p-6">
                {}
                <div class="flex items-center gap-2 mb-3">
                    <span class="px-2 py-1 text-xs font-medium rounded {}">{:?}</span>
                    {}
                    {}
                </div>
                <h3 class="text-lg font-semibold text-gray-900 mb-2">{}</h3>
                <div class="text-sm text-gray-600 space-y-2">{}</div>
                <p class="text-xs text-gray-400 mt-4">{}</p>
            </div>"#,
            image_html,
            type_badge_color,
            announcement.announcement_type,
            visibility_badge,
            featured_badge,
            crate::web::escape_html(&announcement.title),
            // Body is authored in Markdown; render to sanitized safe-subset
            // HTML (already-safe, injected raw). Block markup replaces the
            // old whitespace-pre-wrap on the removed <p>.
            crate::util::markdown::render_announcement_markdown(&announcement.content),
            published_date,
        ));
    }

    html.push_str("</div>");
    axum::response::Html(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use chrono::Utc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::{
        domain::{Announcement, AnnouncementType, CreateMemberRequest, Member},
        repository::{MemberRepository, SqliteAnnouncementRepository, SqliteMemberRepository},
    };

    async fn migrated_pool() -> sqlx::SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    // Regression: the "Show all" checkbox serializes as `show_all=on`.
    // When `AnnouncementsListQuery.show_all` was `Option<bool>`,
    // serde_urlencoded could not parse "on", so the `Query` extractor 400'd
    // and the member-content announcements fragment broke whenever the box
    // was checked. It must now return 200 and render the list fragment.
    #[tokio::test]
    async fn show_all_on_returns_list_fragment() {
        let pool = migrated_pool().await;

        let member_repo: Arc<dyn MemberRepository> =
            Arc::new(SqliteMemberRepository::new(pool.clone()));
        let member: Member = member_repo
            .create(CreateMemberRequest {
                email: "member@example.com".to_string(),
                username: "member".to_string(),
                full_name: "Member".to_string(),
                password: "p4ssword_long_enough".to_string(),
                membership_type_id: None,
                ..Default::default()
            })
            .await
            .unwrap();

        let announcement_repo: Arc<dyn AnnouncementRepository> =
            Arc::new(SqliteAnnouncementRepository::new(pool.clone()));
        let now = Utc::now();
        announcement_repo
            .create(Announcement {
                id: Uuid::new_v4(),
                title: "Hello Members".to_string(),
                content: "Body".to_string(),
                announcement_type: AnnouncementType::General,
                announcement_type_id: None,
                is_public: false,
                featured: false,
                image_url: None,
                published_at: Some(now),
                scheduled_publish_at: None,
                scheduled_publish_timezone: "UTC".to_string(),
                created_by: member.id,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let app = Router::new()
            .route(
                "/portal/api/announcements/list",
                get(announcements_list_api),
            )
            .layer(Extension(CurrentUser { member }))
            .with_state(announcement_repo);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/portal/api/announcements/list?show_all=on")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "checkbox `show_all=on` must parse, not 400"
        );

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            body.contains("Hello Members"),
            "list fragment should render the published announcement, got: {body}"
        );
    }
}
