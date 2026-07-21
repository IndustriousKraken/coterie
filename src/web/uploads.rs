use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use sqlx::SqlitePool;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::auth::AuthService;
use crate::config::Settings;
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

/// Inspect the first bytes of an uploaded document and return its
/// detected format as a canonical extension string. Currently PDF-only:
/// PPTX/DOCX/other Office formats are ZIP containers sharing `PK\x03\x04`
/// magic with arbitrary/zip-bomb archives and cannot be safely told apart
/// by sniffing, so they are deliberately out of scope. The client
/// extension / content-type are hints — this magic-byte check is the
/// authoritative decision, exactly like `detect_image_format`.
fn detect_document_format(data: &[u8]) -> Option<&'static str> {
    // PDF: the "%PDF-" signature. The spec allows leading bytes before
    // it, but real-world PDFs start with it; a stricter prefix check is
    // the safer default here (a file that only carries %PDF- deeper in is
    // more likely a polyglot than a legitimate document).
    if data.starts_with(b"%PDF-") {
        return Some("pdf");
    }
    None
}

/// Save an uploaded document (PDF only) to the uploads directory,
/// reusing the generated-name + size-cap behavior of
/// [`save_uploaded_file`]. The content is confirmed by magic-byte sniff;
/// the client-supplied filename/extension never influences acceptance or
/// the storage path. Returns the relative path (e.g. `uploads/abc123.pdf`).
pub async fn save_uploaded_document(uploads_dir: &str, data: &[u8]) -> Result<String> {
    if data.len() > MAX_FILE_SIZE {
        return Err(AppError::Validation("File too large (max 10 MB)".to_string()));
    }

    // Authoritative sniff — SVG and every non-PDF type stay rejected.
    let detected = detect_document_format(data).ok_or_else(|| {
        AppError::Validation("File content is not a recognized PDF document.".to_string())
    })?;

    let uploads_path = PathBuf::from(uploads_dir);
    fs::create_dir_all(&uploads_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create uploads directory: {}", e)))?;

    // Server-generated, high-entropy name — the uploader's filename can
    // never influence the storage path (no traversal) and a leaked path
    // isn't itself an authorization bypass (the gated route is the
    // control).
    let new_filename = format!("{}.{}", Uuid::new_v4(), detected);
    let file_path = uploads_path.join(&new_filename);

    let mut file = fs::File::create(&file_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create file: {}", e)))?;
    file.write_all(data)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write file: {}", e)))?;

    Ok(format!("uploads/{}", new_filename))
}

/// Save an uploaded file to the uploads directory.
/// Returns the relative path to the file (e.g., "uploads/abc123.jpg")
pub async fn save_uploaded_file(uploads_dir: &str, filename: &str, data: &[u8]) -> Result<String> {
    // Validate file size
    if data.len() > MAX_FILE_SIZE {
        return Err(AppError::Validation(
            "File too large (max 10 MB)".to_string(),
        ));
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
            "File content is not a recognized image (JPEG, PNG, GIF, or WebP).".to_string(),
        )
    })?;

    // Also require the extension to match the detected format — not a
    // security issue on its own (we'll serve what the file actually is)
    // but catches mismatches up front rather than letting them confuse
    // downstream consumers. `jpeg` is accepted as an alias for `jpg`.
    let ext_canonical = if extension == "jpeg" {
        "jpg"
    } else {
        extension.as_str()
    };
    if detected != ext_canonical {
        return Err(AppError::Validation(format!(
            "File extension .{} doesn't match actual format ({}).",
            extension, detected
        )));
    }

    // Ensure uploads directory exists
    let uploads_path = PathBuf::from(uploads_dir);
    fs::create_dir_all(&uploads_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create uploads directory: {}", e)))?;

    // Generate unique filename
    let new_filename = format!("{}.{}", Uuid::new_v4(), extension);
    let file_path = uploads_path.join(&new_filename);

    // Write file
    let mut file = fs::File::create(&file_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to create file: {}", e)))?;

    file.write_all(data)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to write file: {}", e)))?;

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
async fn is_private_image(db_pool: &SqlitePool, image_path: &str) -> bool {
    let full_path = format!("uploads/{}", image_path);

    // Check if used by a private event
    let event_private: Option<(i32,)> = sqlx::query_as(
        r#"
        SELECT 1 FROM events
        WHERE image_url = ? AND visibility != 'Public'
        LIMIT 1
        "#,
    )
    .bind(&full_path)
    .fetch_optional(db_pool)
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
        "#,
    )
    .bind(&full_path)
    .fetch_optional(db_pool)
    .await
    .ok()
    .flatten();

    announcement_private.is_some()
}

/// Serve uploaded files with authentication check for private content
pub async fn serve_upload(
    State(settings): State<Arc<Settings>>,
    State(db_pool): State<SqlitePool>,
    State(auth_service): State<Arc<AuthService>>,
    jar: CookieJar,
    Path(filename): Path<String>,
) -> Response {
    // Validate filename (prevent path traversal)
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // Check if this is a private image
    if is_private_image(&db_pool, &filename).await {
        // Require authentication
        let is_authenticated = if let Some(session_cookie) = jar.get("session") {
            auth_service
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
    let uploads_dir = settings.server.uploads_path();
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

    (StatusCode::OK, [(header::CONTENT_TYPE, content_type)], body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_real_images() {
        assert_eq!(
            detect_image_format(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0]),
            Some("jpg")
        );
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

    #[test]
    fn detects_pdf_by_magic() {
        assert_eq!(detect_document_format(b"%PDF-1.7\n..."), Some("pdf"));
        assert_eq!(detect_document_format(b"%PDF-1.4 rest"), Some("pdf"));
    }

    #[test]
    fn rejects_non_pdf_documents() {
        // A PNG is a valid image but not a document.
        assert_eq!(
            detect_document_format(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            None
        );
        // A ZIP (PK\x03\x04) renamed .pdf — the whole reason PPTX/DOCX
        // are deferred. Must NOT sniff as PDF.
        assert_eq!(detect_document_format(b"PK\x03\x04 zip payload"), None);
        // SVG is active content and stays rejected.
        assert_eq!(detect_document_format(b"<svg xmlns=..."), None);
        assert_eq!(detect_document_format(b""), None);
        // %PDF- appearing later (polyglot) is not accepted — prefix only.
        assert_eq!(detect_document_format(b"GIF89a%PDF-"), None);
    }

    #[tokio::test]
    async fn oversized_pdf_is_rejected_before_write() {
        // A valid-signature PDF over the 10 MB cap is refused (the size
        // check runs before any filesystem write, so the dir is irrelevant).
        let mut data = Vec::from(*b"%PDF-1.7");
        data.resize(MAX_FILE_SIZE + 1, b'0');
        let err = save_uploaded_document("/tmp/coterie-nonexistent-uploads", &data)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }
}
