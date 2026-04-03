use std::path::PathBuf;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::api::state::AppState;
use crate::error::{AppError, Result};

/// Allowed image extensions
const ALLOWED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp"];

/// Maximum file size (10 MB)
const MAX_FILE_SIZE: usize = 10 * 1024 * 1024;

/// Save an uploaded file to the uploads directory.
/// Returns the relative path to the file (e.g., "uploads/abc123.jpg")
pub async fn save_uploaded_file(
    uploads_dir: &str,
    filename: &str,
    data: &[u8],
) -> Result<String> {
    // Validate file size
    if data.len() > MAX_FILE_SIZE {
        return Err(AppError::Validation("File too large (max 10 MB)".to_string()));
    }

    // Extract and validate extension
    let extension = filename
        .rsplit('.')
        .next()
        .map(|s| s.to_lowercase())
        .ok_or_else(|| AppError::Validation("Invalid filename".to_string()))?;

    if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
        return Err(AppError::Validation(format!(
            "Invalid file type. Allowed: {}",
            ALLOWED_EXTENSIONS.join(", ")
        )));
    }

    // Ensure uploads directory exists
    let uploads_path = PathBuf::from(uploads_dir);
    fs::create_dir_all(&uploads_path).await.map_err(|e| {
        AppError::Internal(format!("Failed to create uploads directory: {}", e))
    })?;

    // Generate unique filename
    let new_filename = format!("{}.{}", Uuid::new_v4(), extension);
    let file_path = uploads_path.join(&new_filename);

    // Write file
    let mut file = fs::File::create(&file_path).await.map_err(|e| {
        AppError::Internal(format!("Failed to create file: {}", e))
    })?;

    file.write_all(data).await.map_err(|e| {
        AppError::Internal(format!("Failed to write file: {}", e))
    })?;

    // Return relative path for storing in database
    Ok(format!("uploads/{}", new_filename))
}

/// Delete an uploaded file by its URL path (e.g., "uploads/abc123.jpg")
#[allow(dead_code)]
pub async fn delete_uploaded_file(url_path: &str) -> Result<()> {
    // Only process local uploads (starts with "uploads/")
    if !url_path.starts_with("uploads/") {
        return Ok(());
    }

    let path = PathBuf::from(url_path);
    if path.exists() {
        fs::remove_file(&path).await.map_err(|e| {
            AppError::Internal(format!("Failed to delete file: {}", e))
        })?;
    }

    Ok(())
}

/// Check if an image requires authentication (used by private event/announcement)
async fn is_private_image(state: &AppState, image_path: &str) -> bool {
    let full_path = format!("uploads/{}", image_path);

    // Check if used by a private event
    let event_private: Option<(i32,)> = sqlx::query_as(
        r#"
        SELECT 1 FROM events
        WHERE image_url = ? AND visibility != 'Public'
        LIMIT 1
        "#
    )
    .bind(&full_path)
    .fetch_optional(&state.service_context.db_pool)
    .await
    .ok()
    .flatten();

    if event_private.is_some() {
        return true;
    }

    // Check if used by a private announcement
    let announcement_private: Option<(i32,)> = sqlx::query_as(
        r#"
        SELECT 1 FROM announcements
        WHERE image_url = ? AND is_public = 0
        LIMIT 1
        "#
    )
    .bind(&full_path)
    .fetch_optional(&state.service_context.db_pool)
    .await
    .ok()
    .flatten();

    announcement_private.is_some()
}

/// Serve uploaded files with authentication check for private content
pub async fn serve_upload(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(filename): Path<String>,
) -> Response {
    // Validate filename (prevent path traversal)
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // Check if this is a private image
    if is_private_image(&state, &filename).await {
        // Require authentication
        let is_authenticated = if let Some(session_cookie) = jar.get("session") {
            state.service_context.auth_service
                .validate_session(session_cookie.value())
                .await
                .ok()
                .flatten()
                .is_some()
        } else {
            false
        };

        if !is_authenticated {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    // Build file path
    let uploads_dir = state.settings.server.uploads_path();
    let file_path = PathBuf::from(&uploads_dir).join(&filename);

    // Check file exists
    if !file_path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Open file
    let file = match fs::File::open(&file_path).await {
        Ok(f) => f,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    // Determine content type
    let content_type = match file_path.extension().and_then(|e| e.to_str()) {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    };

    // Stream file
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, content_type)],
        body,
    ).into_response()
}
