use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Extension,
};

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    repository::MemberRepository,
    service::membership_type_service::MembershipTypeService,
    web::templates::{filters, BaseContext, HtmlTemplate},
};

use super::{AdminMembersQuery, MembershipTypeOption};

#[derive(Template)]
#[template(path = "admin/members.html")]
pub struct AdminMembersTemplate {
    pub base: BaseContext,
    pub members: Vec<AdminMemberInfo>,
    pub total_members: i64,
    pub current_page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub search_query: String,
    pub status_filter: String,
    pub type_filter: String,
    pub type_options: Vec<MembershipTypeOption>,
    pub sort_field: String,
    pub sort_order: String,
}

#[derive(Template)]
#[template(path = "admin/members_table.html")]
pub struct AdminMembersTableTemplate {
    pub members: Vec<AdminMemberInfo>,
    pub total_members: i64,
    pub current_page: i64,
    pub per_page: i64,
    pub total_pages: i64,
    pub search_query: String,
    pub status_filter: String,
    pub type_filter: String,
    pub sort_field: String,
    pub sort_order: String,
}

#[derive(Clone)]
pub struct AdminMemberInfo {
    pub id: uuid::Uuid,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub initials: String,
    pub status: crate::domain::MemberStatus,
    pub membership_type: String,
    pub joined_at: chrono::DateTime<chrono::Utc>,
    pub dues_paid_until: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn admin_members_page(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    headers: axum::http::HeaderMap,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<AdminMembersQuery>,
) -> impl IntoResponse {
    let is_htmx = headers.get("HX-Request").is_some();

    let base = BaseContext::for_member(&csrf_service, &current_user, &session_info).await;

    let page = query.page.unwrap_or(1).max(1);
    let per_page: i64 = 20;
    let offset = (page - 1) * per_page;

    let sort_field = query.sort.clone().unwrap_or_else(|| "name".to_string());
    let sort_order = query.order.clone().unwrap_or_else(|| "asc".to_string());

    // Load all membership types up front: drives the filter dropdown,
    // resolves the URL `?type=<slug>` filter to its FK, and provides
    // the per-row display name.
    let all_types = membership_type_service
        .list(true)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("admin members: list membership types failed: {}", e);
            Vec::new()
        });
    let type_filter_id = query
        .member_type
        .as_deref()
        .and_then(|slug| all_types.iter().find(|t| t.slug == slug).map(|t| t.id));
    let type_name_by_id: std::collections::HashMap<uuid::Uuid, String> =
        all_types.iter().map(|t| (t.id, t.name.clone())).collect();
    let type_options: Vec<MembershipTypeOption> = all_types
        .iter()
        .filter(|t| t.is_active)
        .map(|t| MembershipTypeOption {
            id: t.id.to_string(),
            slug: t.slug.clone(),
            name: t.name.clone(),
        })
        .collect();

    // Parse the wire-shape query into the typed MemberQuery. Unknown
    // sort/order values fall back to the defaults rather than failing
    // the request (the dropdown only offers known values; keeping the
    // page renderable on a stale URL is friendlier). Unknown
    // status/type filter values resolve to None — matches the prior
    // "no filter" behavior on a malformed value.
    use crate::repository::{MemberQuery, MemberSortField, SortOrder};
    let typed_query = MemberQuery {
        search: query.q.clone().filter(|s| !s.is_empty()),
        status: query
            .status
            .as_deref()
            .and_then(crate::domain::MemberStatus::from_str),
        membership_type_id: type_filter_id,
        sort: match sort_field.as_str() {
            "status" => MemberSortField::Status,
            "type" => MemberSortField::MembershipType,
            "joined" => MemberSortField::Joined,
            "dues" => MemberSortField::DuesPaidUntil,
            _ => MemberSortField::Name,
        },
        order: if sort_order == "desc" {
            SortOrder::Desc
        } else {
            SortOrder::Asc
        },
        limit: per_page,
        offset,
    };

    let (members, total_members) = member_repo.search(typed_query).await.unwrap_or_else(|e| {
        tracing::error!("admin members search failed: {}", e);
        (Vec::new(), 0)
    });
    let total_pages = (total_members + per_page - 1) / per_page;

    let paginated_members: Vec<AdminMemberInfo> = members
        .into_iter()
        .map(|m| {
            let initials: String = m
                .full_name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();

            AdminMemberInfo {
                id: m.id,
                email: m.email,
                username: m.username,
                full_name: m.full_name,
                initials: if initials.is_empty() {
                    "?".to_string()
                } else {
                    initials
                },
                status: m.status,
                membership_type: type_name_by_id
                    .get(&m.membership_type_id)
                    .cloned()
                    .unwrap_or_else(|| "(unknown)".to_string()),
                joined_at: m.joined_at,
                dues_paid_until: m.dues_paid_until,
            }
        })
        .collect();

    let search_query_val = query.q.unwrap_or_default();
    let status_filter_val = query.status.unwrap_or_default();
    let type_filter_val = query.member_type.unwrap_or_default();

    if is_htmx {
        HtmlTemplate(AdminMembersTableTemplate {
            members: paginated_members,
            total_members,
            current_page: page,
            per_page,
            total_pages,
            search_query: search_query_val,
            status_filter: status_filter_val,
            type_filter: type_filter_val,
            sort_field,
            sort_order,
        })
        .into_response()
    } else {
        HtmlTemplate(AdminMembersTemplate {
            base,
            members: paginated_members,
            total_members,
            current_page: page,
            per_page,
            type_options,
            total_pages,
            search_query: search_query_val,
            status_filter: status_filter_val,
            type_filter: type_filter_val,
            sort_field,
            sort_order,
        })
        .into_response()
    }
}
