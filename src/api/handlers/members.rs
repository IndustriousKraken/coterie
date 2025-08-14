use axum::{
    extract::{Path, Query, State, Extension},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::{middleware::auth::CurrentUser, state::AppState},
    domain::{CreateMemberRequest, Member, MemberStatus, UpdateMemberRequest},
    error::{AppError, Result},
    repository::MemberRepository,
};

#[derive(Debug, Deserialize)]
pub struct ListParams {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    members: Vec<MemberDto>,
    total: usize,
}

#[derive(Debug, Serialize)]
pub struct MemberDto {
    id: Uuid,
    email: String,
    username: String,
    full_name: String,
    status: MemberStatus,
    joined_at: String,
    expires_at: Option<String>,
}

impl From<Member> for MemberDto {
    fn from(member: Member) -> Self {
        Self {
            id: member.id,
            email: member.email,
            username: member.username,
            full_name: member.full_name,
            status: member.status,
            joined_at: member.joined_at.to_rfc3339(),
            expires_at: member.expires_at.map(|dt| dt.to_rfc3339()),
        }
    }
}

pub async fn list(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Query(params): Query<ListParams>,
) -> Result<Json<ListResponse>> {
    let members = state.service_context.member_repo
        .list(params.limit, params.offset)
        .await?;
    
    let total = members.len();
    let members: Vec<MemberDto> = members.into_iter().map(Into::into).collect();
    
    Ok(Json(ListResponse { members, total }))
}

pub async fn get(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<MemberDto>> {
    let member = state.service_context.member_repo
        .find_by_id(id)
        .await?
        .ok_or_else(|| AppError::NotFound("Member not found".to_string()))?;
    
    Ok(Json(member.into()))
}

#[derive(Debug, Deserialize)]
pub struct CreateMemberDto {
    email: String,
    username: String,
    full_name: String,
    membership_type: String,
    password: String,
}

pub async fn create(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Json(dto): Json<CreateMemberDto>,
) -> Result<(StatusCode, Json<MemberDto>)> {
    // Parse membership type
    let membership_type = serde_json::from_str(&format!("\"{}\"", dto.membership_type))
        .map_err(|_| AppError::BadRequest("Invalid membership type".to_string()))?;
    
    // Create member request
    let request = CreateMemberRequest {
        email: dto.email,
        username: dto.username,
        full_name: dto.full_name,
        membership_type,
    };
    
    // TODO: Hash the password and store it
    // For now, the repository uses a temporary password
    
    let member = state.service_context.member_repo
        .create(request)
        .await?;
    
    Ok((StatusCode::CREATED, Json(member.into())))
}

pub async fn update(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Json(update): Json<UpdateMemberRequest>,
) -> Result<Json<MemberDto>> {
    let member = state.service_context.member_repo
        .update(id, update)
        .await?;
    
    Ok(Json(member.into()))
}

pub async fn delete(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode> {
    state.service_context.member_repo
        .delete(id)
        .await?;
    
    Ok(StatusCode::NO_CONTENT)
}

pub async fn activate(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<MemberDto>> {
    let update = UpdateMemberRequest {
        status: Some(MemberStatus::Active),
        ..Default::default()
    };
    
    let member = state.service_context.member_repo
        .update(id, update)
        .await?;
    
    // Trigger integration events
    use crate::integrations::IntegrationEvent;
    state.service_context.integration_manager
        .handle_event(IntegrationEvent::MemberActivated(member.clone()))
        .await;
    
    Ok(Json(member.into()))
}

pub async fn expire(
    State(state): State<AppState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Result<Json<MemberDto>> {
    let update = UpdateMemberRequest {
        status: Some(MemberStatus::Expired),
        expires_at: Some(chrono::Utc::now()),
        ..Default::default()
    };
    
    let member = state.service_context.member_repo
        .update(id, update)
        .await?;
    
    // Trigger integration events
    use crate::integrations::IntegrationEvent;
    state.service_context.integration_manager
        .handle_event(IntegrationEvent::MemberExpired(member.clone()))
        .await;
    
    Ok(Json(member.into()))
}