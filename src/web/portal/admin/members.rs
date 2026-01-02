use askama::Template;
use axum::{
    extract::{State, Query, Path},
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
};
use crate::web::portal::is_admin;

#[derive(Template)]
#[template(path = "admin/members.html")]
pub struct AdminMembersTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
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
    pub id: String,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub initials: String,
    pub status: String,
    pub membership_type: String,
    pub joined_at: String,
    pub dues_paid_until: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdminMembersQuery {
    pub q: Option<String>,
    pub status: Option<String>,
    #[serde(rename = "type")]
    pub member_type: Option<String>,
    pub page: Option<i64>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

pub async fn admin_members_page(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Query(query): Query<AdminMembersQuery>,
) -> impl IntoResponse {
    let is_htmx = headers.get("HX-Request").is_some();

    if !is_admin(&current_user.member) {
        return HtmlTemplate(AdminMembersTemplate {
            current_user: None,
            is_admin: false,
            csrf_token: String::new(),
            members: vec![],
            total_members: 0,
            current_page: 1,
            per_page: 20,
            total_pages: 0,
            search_query: String::new(),
            status_filter: String::new(),
            type_filter: String::new(),
            sort_field: "name".to_string(),
            sort_order: "asc".to_string(),
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
    let offset = (page - 1) * per_page;

    let all_members = state.service_context.member_repo
        .list(1000, 0)
        .await
        .unwrap_or_default();

    let search_query = query.q.clone().unwrap_or_default().to_lowercase();
    let status_filter = query.status.clone().unwrap_or_default();
    let type_filter = query.member_type.clone().unwrap_or_default();
    let sort_field = query.sort.clone().unwrap_or_else(|| "name".to_string());
    let sort_order = query.order.clone().unwrap_or_else(|| "asc".to_string());

    let mut filtered_members: Vec<_> = all_members.into_iter()
        .filter(|m| {
            if !search_query.is_empty() {
                let matches = m.full_name.to_lowercase().contains(&search_query)
                    || m.email.to_lowercase().contains(&search_query)
                    || m.username.to_lowercase().contains(&search_query);
                if !matches {
                    return false;
                }
            }
            if !status_filter.is_empty() {
                if format!("{:?}", m.status) != status_filter {
                    return false;
                }
            }
            if !type_filter.is_empty() {
                if format!("{:?}", m.membership_type) != type_filter {
                    return false;
                }
            }
            true
        })
        .collect();

    filtered_members.sort_by(|a, b| {
        let cmp = match sort_field.as_str() {
            "name" => {
                let a_parts: Vec<&str> = a.full_name.split_whitespace().collect();
                let b_parts: Vec<&str> = b.full_name.split_whitespace().collect();
                let a_last = a_parts.last().unwrap_or(&"");
                let b_last = b_parts.last().unwrap_or(&"");
                a_last.to_lowercase().cmp(&b_last.to_lowercase())
                    .then_with(|| a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase()))
            }
            "status" => format!("{:?}", a.status).cmp(&format!("{:?}", b.status)),
            "type" => format!("{:?}", a.membership_type).cmp(&format!("{:?}", b.membership_type)),
            "joined" => a.joined_at.cmp(&b.joined_at),
            "dues" => {
                match (&a.dues_paid_until, &b.dues_paid_until) {
                    (Some(a_date), Some(b_date)) => a_date.cmp(b_date),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            }
            _ => a.full_name.to_lowercase().cmp(&b.full_name.to_lowercase()),
        };
        if sort_order == "desc" { cmp.reverse() } else { cmp }
    });

    let total_members = filtered_members.len() as i64;
    let total_pages = (total_members + per_page - 1) / per_page;

    let paginated_members: Vec<AdminMemberInfo> = filtered_members
        .into_iter()
        .skip(offset as usize)
        .take(per_page as usize)
        .map(|m| {
            let initials: String = m.full_name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();

            AdminMemberInfo {
                id: m.id.to_string(),
                email: m.email,
                username: m.username,
                full_name: m.full_name,
                initials: if initials.is_empty() { "?".to_string() } else { initials },
                status: format!("{:?}", m.status),
                membership_type: format!("{:?}", m.membership_type),
                joined_at: m.joined_at.format("%b %d, %Y").to_string(),
                dues_paid_until: m.dues_paid_until.map(|d| d.format("%b %d, %Y").to_string()),
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
        }).into_response()
    } else {
        HtmlTemplate(AdminMembersTemplate {
            current_user: Some(user_info),
            is_admin: true,
            csrf_token,
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
        }).into_response()
    }
}

pub async fn admin_activate_member(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    use crate::domain::{UpdateMemberRequest, MemberStatus};

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<tr><td colspan='6' class='px-6 py-4 text-red-600'>Invalid member ID</td></tr>".to_string()),
    };

    let update = UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        ..Default::default()
    };

    match state.service_context.member_repo.update(id, update).await {
        Ok(member) => {
            let initials: String = member.full_name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();

            axum::response::Html(format!(
                r#"<tr class="hover:bg-gray-50 bg-green-50" x-data="{{ open: false }}">
                    <td class="px-6 py-4 whitespace-nowrap">
                        <div class="flex items-center">
                            <div class="flex-shrink-0 h-10 w-10 bg-gray-200 rounded-full flex items-center justify-center">
                                <span class="text-gray-600 font-medium text-sm">{}</span>
                            </div>
                            <div class="ml-4">
                                <div class="text-sm font-medium text-gray-900">{}</div>
                                <div class="text-sm text-gray-500">{}</div>
                                <div class="text-xs text-gray-400">@{}</div>
                            </div>
                        </div>
                    </td>
                    <td class="px-6 py-4 whitespace-nowrap">
                        <span class="px-2 inline-flex text-xs leading-5 font-semibold rounded-full bg-green-100 text-green-800">Active</span>
                    </td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{:?}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium">
                        <span class="text-green-600">Activated!</span>
                    </td>
                </tr>"#,
                initials,
                member.full_name,
                member.email,
                member.username,
                member.membership_type,
                member.joined_at.format("%b %d, %Y"),
                member.dues_paid_until.map(|d| d.format("%b %d, %Y").to_string()).unwrap_or_else(|| "—".to_string())
            ))
        }
        Err(e) => {
            axum::response::Html(format!(
                "<tr><td colspan='6' class='px-6 py-4 text-red-600'>Error: {}</td></tr>",
                e
            ))
        }
    }
}

pub async fn admin_suspend_member(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    use crate::domain::{UpdateMemberRequest, MemberStatus};

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html("<tr><td colspan='6' class='px-6 py-4 text-red-600'>Invalid member ID</td></tr>".to_string()),
    };

    let update = UpdateMemberRequest {
        status: Some(MemberStatus::Suspended),
        ..Default::default()
    };

    match state.service_context.member_repo.update(id, update).await {
        Ok(member) => {
            let initials: String = member.full_name
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();

            axum::response::Html(format!(
                r#"<tr class="hover:bg-gray-50 bg-yellow-50" x-data="{{ open: false }}">
                    <td class="px-6 py-4 whitespace-nowrap">
                        <div class="flex items-center">
                            <div class="flex-shrink-0 h-10 w-10 bg-gray-200 rounded-full flex items-center justify-center">
                                <span class="text-gray-600 font-medium text-sm">{}</span>
                            </div>
                            <div class="ml-4">
                                <div class="text-sm font-medium text-gray-900">{}</div>
                                <div class="text-sm text-gray-500">{}</div>
                                <div class="text-xs text-gray-400">@{}</div>
                            </div>
                        </div>
                    </td>
                    <td class="px-6 py-4 whitespace-nowrap">
                        <span class="px-2 inline-flex text-xs leading-5 font-semibold rounded-full bg-gray-100 text-gray-800">Suspended</span>
                    </td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{:?}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-sm text-gray-500">{}</td>
                    <td class="px-6 py-4 whitespace-nowrap text-right text-sm font-medium">
                        <span class="text-yellow-600">Suspended</span>
                    </td>
                </tr>"#,
                initials,
                member.full_name,
                member.email,
                member.username,
                member.membership_type,
                member.joined_at.format("%b %d, %Y"),
                member.dues_paid_until.map(|d| d.format("%b %d, %Y").to_string()).unwrap_or_else(|| "—".to_string())
            ))
        }
        Err(e) => {
            axum::response::Html(format!(
                "<tr><td colspan='6' class='px-6 py-4 text-red-600'>Error: {}</td></tr>",
                e
            ))
        }
    }
}

// Member Detail Page

#[derive(Template)]
#[template(path = "admin/member_detail.html")]
pub struct AdminMemberDetailTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
    pub member: AdminMemberDetailInfo,
}

pub struct AdminMemberDetailInfo {
    pub id: String,
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub initials: String,
    pub status: String,
    pub membership_type: String,
    pub joined_at: String,
    pub dues_paid_until: Option<String>,
    pub dues_expired: bool,
    pub bypass_dues: bool,
    pub notes: String,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn admin_member_detail_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    if !is_admin(&current_user.member) {
        return axum::response::Redirect::to("/portal/dashboard").into_response();
    }

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Redirect::to("/portal/admin/members").into_response(),
    };

    let member = match state.service_context.member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Redirect::to("/portal/admin/members").into_response(),
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

    let initials: String = member.full_name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let now = chrono::Utc::now();
    let dues_expired = member.dues_paid_until
        .map(|d| d < now)
        .unwrap_or(true);

    let member_info = AdminMemberDetailInfo {
        id: member.id.to_string(),
        email: member.email,
        username: member.username,
        full_name: member.full_name,
        initials: if initials.is_empty() { "?".to_string() } else { initials },
        status: format!("{:?}", member.status),
        membership_type: format!("{:?}", member.membership_type),
        joined_at: member.joined_at.format("%B %d, %Y").to_string(),
        dues_paid_until: member.dues_paid_until.map(|d| d.format("%B %d, %Y").to_string()),
        dues_expired,
        bypass_dues: member.bypass_dues,
        notes: member.notes.unwrap_or_default(),
        created_at: member.created_at.format("%B %d, %Y").to_string(),
        updated_at: member.updated_at.format("%B %d, %Y at %l:%M %p").to_string(),
    };

    let template = AdminMemberDetailTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
        member: member_info,
    };

    HtmlTemplate(template).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AdminUpdateMemberForm {
    pub full_name: String,
    pub membership_type: String,
    pub notes: Option<String>,
    pub bypass_dues: Option<String>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_update_member(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<AdminUpdateMemberForm>,
) -> impl IntoResponse {
    use crate::domain::{UpdateMemberRequest, MembershipType};

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Invalid member ID</div>"#.to_string()
        ),
    };

    let membership_type = match form.membership_type.as_str() {
        "Regular" => MembershipType::Regular,
        "Student" => MembershipType::Student,
        "Corporate" => MembershipType::Corporate,
        "Lifetime" => MembershipType::Lifetime,
        _ => MembershipType::Regular,
    };

    let update = UpdateMemberRequest {
        full_name: Some(form.full_name),
        membership_type: Some(membership_type),
        notes: Some(form.notes.unwrap_or_default()),
        bypass_dues: Some(form.bypass_dues.is_some()),
        ..Default::default()
    };

    match state.service_context.member_repo.update(id, update).await {
        Ok(_) => axum::response::Html(
            r#"<div class="p-3 bg-green-50 text-green-800 rounded-md text-sm">Member updated successfully!</div>"#.to_string()
        ),
        Err(e) => axum::response::Html(format!(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Error: {}</div>"#,
            e
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct ExtendDuesForm {
    pub months: i32,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_extend_dues(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<ExtendDuesForm>,
) -> impl IntoResponse {
    use chrono::Months;

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Invalid member ID</div>"#.to_string()
        ),
    };

    let member = match state.service_context.member_repo.find_by_id(id).await {
        Ok(Some(m)) => m,
        _ => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Member not found</div>"#.to_string()
        ),
    };

    let now = chrono::Utc::now();
    let base_date = member.dues_paid_until
        .filter(|d| *d > now)
        .unwrap_or(now);

    let new_dues_date = base_date
        .checked_add_months(Months::new(form.months as u32))
        .unwrap_or(base_date);

    let result = sqlx::query("UPDATE members SET dues_paid_until = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(new_dues_date)
        .bind(member_id)
        .execute(&state.service_context.db_pool)
        .await;

    match result {
        Ok(_) => axum::response::Html(format!(
            r#"<div class="p-3 bg-green-50 text-green-800 rounded-md text-sm">
                Dues extended! New expiration: {}
                <script>setTimeout(() => location.reload(), 1500)</script>
            </div>"#,
            new_dues_date.format("%B %d, %Y")
        )),
        Err(e) => axum::response::Html(format!(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Error: {}</div>"#,
            e
        )),
    }
}

#[derive(Debug, Deserialize)]
pub struct SetDuesForm {
    pub dues_until: String,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_set_dues(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    axum::Form(form): axum::Form<SetDuesForm>,
) -> impl IntoResponse {
    use chrono::NaiveDate;

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Invalid member ID</div>"#.to_string()
        ),
    };

    let naive_date = match NaiveDate::parse_from_str(&form.dues_until, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Invalid date format</div>"#.to_string()
        ),
    };

    let dues_date = naive_date
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc();

    let result = sqlx::query("UPDATE members SET dues_paid_until = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(dues_date)
        .bind(id.to_string())
        .execute(&state.service_context.db_pool)
        .await;

    match result {
        Ok(_) => axum::response::Html(format!(
            r#"<div class="p-3 bg-green-50 text-green-800 rounded-md text-sm">
                Dues date set to: {}
                <script>setTimeout(() => location.reload(), 1500)</script>
            </div>"#,
            dues_date.format("%B %d, %Y")
        )),
        Err(e) => axum::response::Html(format!(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Error: {}</div>"#,
            e
        )),
    }
}

pub async fn admin_expire_now(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Invalid member ID</div>"#.to_string()
        ),
    };

    let yesterday = chrono::Utc::now() - chrono::Duration::days(1);

    let result = sqlx::query("UPDATE members SET dues_paid_until = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(yesterday)
        .bind(id.to_string())
        .execute(&state.service_context.db_pool)
        .await;

    match result {
        Ok(_) => axum::response::Html(
            r#"<div class="p-3 bg-yellow-50 text-yellow-800 rounded-md text-sm">
                Member dues have been expired.
                <script>setTimeout(() => location.reload(), 1500)</script>
            </div>"#.to_string()
        ),
        Err(e) => axum::response::Html(format!(
            r#"<div class="p-3 bg-red-50 text-red-800 rounded-md text-sm">Error: {}</div>"#,
            e
        )),
    }
}

pub async fn admin_member_payments(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> impl IntoResponse {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return axum::response::Html(
            r#"<div class="p-6 text-center text-red-600">Invalid member ID</div>"#.to_string()
        ),
    };

    let payments = state.service_context.payment_repo
        .find_by_member(id)
        .await
        .unwrap_or_default();

    if payments.is_empty() {
        return axum::response::Html(
            r#"<div class="p-6 text-center text-gray-500">No payment history for this member</div>"#.to_string()
        );
    }

    let mut html = String::new();

    for payment in payments {
        let status_badge = match format!("{:?}", payment.status).as_str() {
            "Completed" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-green-100 text-green-800">Completed</span>"#,
            "Pending" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-yellow-100 text-yellow-800">Pending</span>"#,
            "Failed" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-red-100 text-red-800">Failed</span>"#,
            "Refunded" => r#"<span class="px-2 py-1 text-xs font-medium rounded bg-gray-100 text-gray-800">Refunded</span>"#,
            _ => "",
        };

        let description = if payment.description.is_empty() {
            "Membership dues".to_string()
        } else {
            payment.description.clone()
        };

        html.push_str(&format!(
            r#"<div class="px-6 py-4 flex justify-between items-center">
                <div>
                    <p class="font-medium text-gray-900">{}</p>
                    <p class="text-sm text-gray-500">{}</p>
                </div>
                <div class="text-right">
                    <p class="font-medium text-gray-900">${:.2}</p>
                    <div class="mt-1">{}</div>
                </div>
            </div>"#,
            description,
            payment.created_at.format("%B %d, %Y"),
            payment.amount_cents as f64 / 100.0,
            status_badge
        ));
    }

    axum::response::Html(html)
}

// New Member Page

#[derive(Template)]
#[template(path = "admin/member_new.html")]
pub struct AdminNewMemberTemplate {
    pub current_user: Option<UserInfo>,
    pub is_admin: bool,
    pub csrf_token: String,
}

pub async fn admin_new_member_page(
    State(state): State<AppState>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> axum::response::Response {
    if !is_admin(&current_user.member) {
        return axum::response::Redirect::to("/portal/dashboard").into_response();
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

    let template = AdminNewMemberTemplate {
        current_user: Some(user_info),
        is_admin: true,
        csrf_token,
    };

    HtmlTemplate(template).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AdminCreateMemberForm {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub membership_type: String,
    pub status: String,
    pub notes: Option<String>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_create_member(
    State(state): State<AppState>,
    Extension(_current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<AdminCreateMemberForm>,
) -> axum::response::Response {
    use crate::domain::{CreateMemberRequest, MembershipType, MemberStatus, UpdateMemberRequest};

    let membership_type = match form.membership_type.as_str() {
        "Regular" => MembershipType::Regular,
        "Student" => MembershipType::Student,
        "Corporate" => MembershipType::Corporate,
        "Lifetime" => MembershipType::Lifetime,
        _ => MembershipType::Regular,
    };

    let create_request = CreateMemberRequest {
        email: form.email.clone(),
        username: form.username.clone(),
        full_name: form.full_name.clone(),
        password: form.password,
        membership_type,
    };

    match state.service_context.member_repo.create(create_request).await {
        Ok(member) => {
            let status = match form.status.as_str() {
                "Active" => Some(MemberStatus::Active),
                "Expired" => Some(MemberStatus::Expired),
                "Suspended" => Some(MemberStatus::Suspended),
                "Honorary" => Some(MemberStatus::Honorary),
                _ => None,
            };

            if status.is_some() || form.notes.is_some() {
                let update = UpdateMemberRequest {
                    status,
                    notes: form.notes,
                    ..Default::default()
                };
                let _ = state.service_context.member_repo.update(member.id, update).await;
            }

            axum::response::Redirect::to(&format!("/portal/admin/members/{}", member.id)).into_response()
        }
        Err(e) => {
            axum::response::Html(format!(
                r#"<!DOCTYPE html>
                <html>
                <head>
                    <title>Error - Coterie</title>
                    <script src="https://cdn.tailwindcss.com"></script>
                </head>
                <body class="bg-gray-100 min-h-screen flex items-center justify-center">
                    <div class="bg-white p-8 rounded-lg shadow-md max-w-md">
                        <h1 class="text-xl font-bold text-red-600 mb-4">Error Creating Member</h1>
                        <p class="text-gray-700 mb-4">{}</p>
                        <a href="/portal/admin/members/new" class="text-blue-600 hover:underline">Go back and try again</a>
                    </div>
                </body>
                </html>"#,
                e
            )).into_response()
        }
    }
}
