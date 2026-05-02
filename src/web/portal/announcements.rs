use askama::Template;
use axum::{
    extract::{State, Query},
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;

use crate::{
    api::{
        middleware::auth::{CurrentUser, SessionInfo},
        state::AppState,
    },
    web::templates::{BaseContext, HtmlTemplate},
};

#[derive(Template)]
#[template(path = "portal/announcements.html")]
pub struct AnnouncementsTemplate {
    pub base: BaseContext,
}

pub async fn announcements_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session): Extension<SessionInfo>,
) -> impl IntoResponse {
    let template = AnnouncementsTemplate {
        base: BaseContext::for_member(&state, &current_user, &session).await,
    };

    HtmlTemplate(template)
}

#[derive(Debug, Deserialize)]
pub struct AnnouncementsListQuery {
    pub announcement_type: Option<String>,
    pub show_all: Option<bool>,
}

pub async fn announcements_list_api(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Query(query): Query<AnnouncementsListQuery>,
) -> impl IntoResponse {
    // Get all published announcements (both public and private - members can see all)
    let limit = if query.show_all.unwrap_or(false) { 100 } else { 20 };
    let announcements = state.service_context.announcement_repo
        .list_recent(limit)
        .await
        .unwrap_or_default();

    // Filter by type if specified
    let filtered_announcements: Vec<_> = announcements.into_iter()
        .filter(|a| {
            if let Some(ref announcement_type) = query.announcement_type {
                if !announcement_type.is_empty() && format!("{:?}", a.announcement_type) != *announcement_type {
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
            </div>"#.to_string()
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

        let published_date = announcement.published_at
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
                <p class="text-sm text-gray-600 whitespace-pre-wrap">{}</p>
                <p class="text-xs text-gray-400 mt-4">{}</p>
            </div>"#,
            image_html,
            type_badge_color,
            announcement.announcement_type,
            visibility_badge,
            featured_badge,
            crate::web::escape_html(&announcement.title),
            crate::web::escape_html(&announcement.content),
            published_date,
        ));
    }

    html.push_str("</div>");
    axum::response::Html(html)
}
