use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension,
};

use crate::{
    api::middleware::auth::CurrentUser, repository::MemberRepository,
    service::member_service::MemberService,
};

/// Admin-triggered: regenerate a verification token for an unverified
/// member and email them the fresh link. Invalidates any previously
/// outstanding tokens so the old email (if the member still has it)
/// can't be used.
pub async fn admin_resend_verification(
    State(member_repo): State<Arc<dyn MemberRepository>>,
    State(member_service): State<Arc<MemberService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
) -> axum::response::Response {
    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return resend_result(false, "Invalid member ID").into_response(),
    };

    // The service refetches the member to render the success message;
    // for the resend flow we need the member's email for the success
    // string, so re-fetch here too. The service-level audit fires on
    // success of the email send.
    let email = match member_repo.find_by_id(id).await {
        Ok(Some(m)) => m.email,
        Ok(None) => return resend_result(false, "Member not found").into_response(),
        Err(e) => return resend_result(false, &format!("DB error: {}", e)).into_response(),
    };

    match member_service
        .resend_verification(current_user.member.id, id)
        .await
    {
        Ok(()) => {
            resend_result(true, &format!("Verification email resent to {}.", email)).into_response()
        }
        Err(e) => resend_result(false, &format!("Send failed: {}", e)).into_response(),
    }
}

fn resend_result(ok: bool, detail: &str) -> axum::response::Html<String> {
    let escaped = crate::web::escape_html(detail);
    let (bg, fg) = if ok {
        ("bg-green-50", "text-green-900")
    } else {
        ("bg-red-50", "text-red-900")
    };
    axum::response::Html(format!(
        r#"<div id="verify-resend-result" class="mt-2 p-2 {bg} {fg} rounded text-sm">{detail}</div>"#,
        bg = bg,
        fg = fg,
        detail = escaped,
    ))
}
