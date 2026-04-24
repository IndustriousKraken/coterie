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

/// Inspect the first bytes of an image and return its detected format
/// as a canonical extension string ("jpg", "png", "gif", "webp"). Any
/// other content returns `None`. The extension alone is a hint from the
/// uploader — this is the authoritative check.
fn detect_image_format(data: &[u8]) -> Option<&'static str> {
    // JPEG: FF D8 FF (all three bytes)
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpg");
    }
    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("png");
    }
    // GIF: "GIF87a" or "GIF89a"
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Some("gif");
    }
    // WebP: RIFF....WEBP (4 bytes, then 4-byte size, then "WEBP")
    if data.len() >= 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("webp");
    }
    None
}

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

    // Magic-byte check: confirm the file actually IS the image type it
    // claims to be. Prevents someone from uploading a zip, script, or
    // HTML file renamed with a .jpg extension (which would then be
    // served with image/jpeg Content-Type and could still be abused in
    // some contexts even with nosniff).
    let detected = detect_image_format(data).ok_or_else(|| {
        AppError::Validation(
            "File content is not a recognized image (JPEG, PNG, GIF, or WebP).".to_string()
        )
    })?;

    // Also require the extension to match the detected format — not a
    // security issue on its own (we'll serve what the file actually is)
    // but catches mismatches up front rather than letting them confuse
    // downstream consumers. `jpeg` is accepted as an alias for `jpg`.
    let ext_canonical = if extension == "jpeg" { "jpg" } else { extension.as_str() };
    if detected != ext_canonical {
        return Err(AppError::Validation(format!(
            "File extension .{} doesn't match actual format ({}).",
            extension, detected
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

/// Delete an uploaded file by its URL path (e.g., "uploads/abc123.jpg").
/// No-op if the path doesn't match our upload convention, the filename
/// is empty, or the file simply doesn't exist.
///
/// `uploads_dir` is the configured filesystem root (from
/// `ServerConfig::uploads_path()`); we join the filename onto it to
/// find the actual file. Path traversal is blocked — any "." or ".."
/// segments or absolute paths make this no-op.
pub async fn delete_uploaded_file(uploads_dir: &str, url_path: &str) -> Result<()> {
    // Strip the "uploads/" URL prefix. Anything else isn't one of ours.
    let filename = match url_path.strip_prefix("uploads/") {
        Some(f) => f,
        None => return Ok(()),
    };
    // Defense: refuse any kind of path trickery.
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
    {
        return Ok(());
    }

    let path = PathBuf::from(uploads_dir).join(filename);
    if path.exists() {
        if let Err(e) = fs::remove_file(&path).await {
            // Don't fail the caller — the DB-level delete already
            // succeeded. Log so orphans don't accumulate silently.
            tracing::warn!("Failed to delete upload {}: {}", path.display(), e);
        }
    }

    Ok(())
}

/// Convenience: delete an upload if `url` is Some and matches our
/// upload convention. No-op on None or non-upload URLs.
pub async fn delete_if_upload(uploads_dir: &str, url: Option<&str>) {
    if let Some(u) = url {
        let _ = delete_uploaded_file(uploads_dir, u).await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_real_images() {
        assert_eq!(detect_image_format(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0]), Some("jpg"));
        assert_eq!(
            detect_image_format(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0]),
            Some("png")
        );
        assert_eq!(detect_image_format(b"GIF87a trailing"), Some("gif"));
        assert_eq!(detect_image_format(b"GIF89a trailing"), Some("gif"));

        // RIFF + 4-byte size + WEBP
        let mut webp = Vec::from(*b"RIFF");
        webp.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
        webp.extend_from_slice(b"WEBP more");
        assert_eq!(detect_image_format(&webp), Some("webp"));
    }

    #[test]
    fn rejects_non_images() {
        // Renamed zip
        assert_eq!(detect_image_format(b"PK\x03\x04 and more zip stuff"), None);
        // Renamed HTML
        assert_eq!(detect_image_format(b"<!DOCTYPE html><html>..."), None);
        // Plain text
        assert_eq!(detect_image_format(b"hello world"), None);
        // Empty
        assert_eq!(detect_image_format(b""), None);
        // Two bytes only
        assert_eq!(detect_image_format(&[0xFF, 0xD8]), None);
    }

    #[test]
    fn partial_riff_not_webp() {
        // RIFF but not WEBP (e.g., WAV, AVI)
        let mut riff_wav = Vec::from(*b"RIFF");
        riff_wav.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
        riff_wav.extend_from_slice(b"WAVE");
        assert_eq!(detect_image_format(&riff_wav), None);
    }
}
