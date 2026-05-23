use std::sync::Arc;

use askama::Template;
use axum::{extract::State, response::IntoResponse, Extension};
use serde::Deserialize;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    service::{member_service::MemberService, membership_type_service::MembershipTypeService},
    web::templates::{BaseContext, HtmlTemplate},
};

use super::MembershipTypeOption;

#[derive(Template)]
#[template(path = "admin/member_new.html")]
pub struct AdminNewMemberTemplate {
    pub base: BaseContext,
    pub type_options: Vec<MembershipTypeOption>,
}

pub async fn admin_new_member_page(
    State(membership_type_service): State<Arc<MembershipTypeService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> axum::response::Response {
    let type_options: Vec<MembershipTypeOption> = membership_type_service
        .list(false)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| MembershipTypeOption {
            id: t.id.to_string(),
            slug: t.slug,
            name: t.name,
        })
        .collect();

    let template = AdminNewMemberTemplate {
        base: BaseContext::for_member(&csrf_service, &current_user, &session_info).await,
        type_options,
    };

    HtmlTemplate(template).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AdminCreateMemberForm {
    pub email: String,
    pub username: String,
    pub full_name: String,
    pub password: String,
    pub membership_type_id: String,
    pub status: String,
    pub notes: Option<String>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_create_member(
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    axum::Form(form): axum::Form<AdminCreateMemberForm>,
) -> axum::response::Response {
    use crate::domain::{CreateMemberRequest, MemberStatus, UpdateMemberRequest};

    fn render_error(message: &str) -> axum::response::Response {
        axum::response::Html(format!(
            r#"<!DOCTYPE html>
            <html>
            <head>
                <title>Error - Coterie</title>
                <link rel="stylesheet" href="/static/style.css">
            </head>
            <body class="bg-gray-100 min-h-screen flex items-center justify-center">
                <div class="bg-white p-8 rounded-lg shadow-md max-w-md">
                    <h1 class="text-xl font-bold text-red-600 mb-4">Error Creating Member</h1>
                    <p class="text-gray-700 mb-4">{}</p>
                    <a href="/portal/admin/members/new" class="text-blue-600 hover:underline">Go back and try again</a>
                </div>
            </body>
            </html>"#,
            crate::web::escape_html(message),
        )).into_response()
    }

    let membership_type_id = match uuid::Uuid::parse_str(&form.membership_type_id) {
        Ok(id) => id,
        Err(_) => return render_error("Invalid membership type."),
    };

    let create_request = CreateMemberRequest {
        email: form.email.clone(),
        username: form.username.clone(),
        full_name: form.full_name.clone(),
        password: form.password,
        membership_type_id: Some(membership_type_id),
        ..Default::default()
    };

    match member_service
        .create(current_user.member.id, create_request)
        .await
    {
        Ok(member) => {
            // Pending is the default already set by `create`, so an
            // empty / "Pending" form value is a no-op — only override
            // when the admin picked a different status. Unknown values
            // (typo, forged form) skip the override rather than silently
            // landing on a default.
            let status = match form.status.as_str() {
                "" | "Pending" => None,
                s => MemberStatus::from_str(s),
            };

            if status.is_some() || form.notes.is_some() {
                let update = UpdateMemberRequest {
                    status,
                    notes: form.notes,
                    ..Default::default()
                };
                if let Err(e) = member_service
                    .update(current_user.member.id, member.id, update)
                    .await
                {
                    // Member was created but the status/notes follow-up
                    // failed. The admin will see the detail page with
                    // the original (Pending, no notes) state — not
                    // catastrophic but worth logging so they know why
                    // the form values didn't take.
                    tracing::error!(
                        "Created member {} but follow-up status/notes update failed: {}",
                        member.id,
                        e
                    );
                }
            }

            axum::response::Redirect::to(&format!("/portal/admin/members/{}", member.id))
                .into_response()
        }
        Err(e) => render_error(&e.to_string()),
    }
}
