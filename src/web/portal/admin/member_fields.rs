//! Admin management of org-defined member fields
//! (`/portal/admin/settings/member-fields`) — see the
//! member-custom-fields capability spec. Value editing lives on the
//! member detail page (admins) and the profile page (members).

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Extension, Form,
};
use serde::Deserialize;

use crate::{
    api::middleware::auth::{CurrentUser, SessionInfo},
    auth::CsrfService,
    domain::{CreateMemberFieldRequest, FieldWithValue, UpdateMemberFieldRequest},
    service::member_field_service::MemberFieldService,
    web::templates::{BaseContext, HtmlTemplate},
};

/// Row shape shared by the management page, the admin member card, and
/// the profile card.
#[derive(Clone)]
pub struct FieldRow {
    pub id: String,
    pub name: String,
    pub field_key: String,
    pub field_type: String,
    pub member_editable: bool,
    pub sort_order: i32,
    pub is_active: bool,
    /// Set for the value-editing cards; empty on the management page.
    pub value: String,
}

pub fn field_rows_with_values(fields: &[FieldWithValue]) -> Vec<FieldRow> {
    fields
        .iter()
        .map(|f| FieldRow {
            id: f.definition.id.to_string(),
            name: f.definition.name.clone(),
            field_key: f.definition.field_key.clone(),
            field_type: f.definition.field_type.as_str().to_string(),
            member_editable: f.definition.member_editable,
            sort_order: f.definition.sort_order,
            is_active: f.definition.is_active,
            value: f.value.clone().unwrap_or_default(),
        })
        .collect()
}

#[derive(Template)]
#[template(path = "admin/member_fields.html")]
pub struct AdminMemberFieldsTemplate {
    pub base: BaseContext,
    pub fields: Vec<FieldRow>,
    pub success_message: Option<String>,
    pub error_message: Option<String>,
}

async fn render_page(
    member_field_service: &MemberFieldService,
    csrf_service: &CsrfService,
    current_user: &CurrentUser,
    session_info: &SessionInfo,
    success_message: Option<String>,
    error_message: Option<String>,
) -> Response {
    let fields = match member_field_service.list_definitions(true).await {
        Ok(defs) => defs
            .into_iter()
            .map(|d| FieldRow {
                id: d.id.to_string(),
                name: d.name,
                field_key: d.field_key,
                field_type: d.field_type.as_str().to_string(),
                member_editable: d.member_editable,
                sort_order: d.sort_order,
                is_active: d.is_active,
                value: String::new(),
            })
            .collect(),
        Err(e) => {
            tracing::error!("Failed to list member field definitions: {}", e);
            Vec::new()
        }
    };

    HtmlTemplate(AdminMemberFieldsTemplate {
        base: BaseContext::for_member(csrf_service, current_user, session_info).await,
        fields,
        success_message,
        error_message,
    })
    .into_response()
}

pub async fn admin_member_fields_page(
    State(member_field_service): State<Arc<MemberFieldService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
) -> Response {
    render_page(
        &member_field_service,
        &csrf_service,
        &current_user,
        &session_info,
        None,
        None,
    )
    .await
}

#[derive(Debug, Deserialize)]
pub struct CreateFieldForm {
    pub name: String,
    pub field_key: Option<String>,
    pub field_type: String,
    pub member_editable: Option<String>,
    pub sort_order: Option<i32>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_create_member_field(
    State(member_field_service): State<Arc<MemberFieldService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Form(form): Form<CreateFieldForm>,
) -> Response {
    let request = CreateMemberFieldRequest {
        name: form.name,
        field_key: form.field_key.filter(|s| !s.trim().is_empty()),
        field_type: form.field_type,
        member_editable: form.member_editable.is_some(),
        sort_order: form.sort_order.unwrap_or(0),
    };
    let (ok, err) = match member_field_service
        .create_definition(current_user.member.id, request)
        .await
    {
        Ok(def) => (Some(format!("Field '{}' created.", def.name)), None),
        Err(e) => (None, Some(format!("Create failed: {}", e))),
    };
    render_page(
        &member_field_service,
        &csrf_service,
        &current_user,
        &session_info,
        ok,
        err,
    )
    .await
}

#[derive(Debug, Deserialize)]
pub struct UpdateFieldForm {
    pub name: String,
    pub field_type: String,
    pub sort_order: i32,
    pub member_editable: Option<String>,
    pub is_active: Option<String>,
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_update_member_field(
    State(member_field_service): State<Arc<MemberFieldService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(id): Path<String>,
    Form(form): Form<UpdateFieldForm>,
) -> Response {
    let (ok, err) = match uuid::Uuid::parse_str(&id) {
        Err(_) => (None, Some("Invalid field ID".to_string())),
        Ok(id) => {
            let request = UpdateMemberFieldRequest {
                name: Some(form.name),
                field_type: Some(form.field_type),
                member_editable: Some(form.member_editable.is_some()),
                sort_order: Some(form.sort_order),
                is_active: Some(form.is_active.is_some()),
            };
            match member_field_service
                .update_definition(current_user.member.id, id, request)
                .await
            {
                Ok(def) => (Some(format!("Field '{}' updated.", def.name)), None),
                Err(e) => (None, Some(format!("Update failed: {}", e))),
            }
        }
    };
    render_page(
        &member_field_service,
        &csrf_service,
        &current_user,
        &session_info,
        ok,
        err,
    )
    .await
}

#[derive(Debug, Deserialize)]
pub struct DeleteFieldForm {
    #[allow(dead_code)]
    pub csrf_token: String,
}

pub async fn admin_delete_member_field(
    State(member_field_service): State<Arc<MemberFieldService>>,
    State(csrf_service): State<Arc<CsrfService>>,
    Extension(current_user): Extension<CurrentUser>,
    Extension(session_info): Extension<SessionInfo>,
    Path(id): Path<String>,
    Form(_form): Form<DeleteFieldForm>,
) -> Response {
    let (ok, err) = match uuid::Uuid::parse_str(&id) {
        Err(_) => (None, Some("Invalid field ID".to_string())),
        Ok(id) => match member_field_service
            .delete_definition(current_user.member.id, id)
            .await
        {
            Ok(()) => (
                Some("Field deleted (stored values removed with it).".to_string()),
                None,
            ),
            Err(e) => (None, Some(format!("Delete failed: {}", e))),
        },
    };
    render_page(
        &member_field_service,
        &csrf_service,
        &current_user,
        &session_info,
        ok,
        err,
    )
    .await
}

/// Extract (key, value) pairs from a dynamic custom-fields form: inputs
/// are named `field_<field_key>`; everything else (csrf_token) is
/// dropped.
pub fn field_pairs(form: &[(String, String)]) -> Vec<(String, String)> {
    form.iter()
        .filter_map(|(k, v)| {
            k.strip_prefix("field_")
                .map(|key| (key.to_string(), v.clone()))
        })
        .collect()
}

/// Admin save of a member's custom field values (member detail page).
pub async fn admin_save_member_custom_fields(
    State(member_field_service): State<Arc<MemberFieldService>>,
    Extension(current_user): Extension<CurrentUser>,
    Path(member_id): Path<String>,
    Form(form): Form<Vec<(String, String)>>,
) -> Response {
    use crate::service::member_field_service::FieldScope;
    use crate::web::portal::admin::partials;

    let id = match uuid::Uuid::parse_str(&member_id) {
        Ok(id) => id,
        Err(_) => return partials::admin_alert("error", "Invalid member ID", false).into_response(),
    };
    let pairs = field_pairs(&form);
    match member_field_service
        .save_values(current_user.member.id, id, &pairs, FieldScope::Admin)
        .await
    {
        Ok(()) => partials::admin_alert("success", "Custom fields saved.", false).into_response(),
        Err(e) => {
            partials::admin_alert("error", &format!("Save failed: {}", e), false).into_response()
        }
    }
}
