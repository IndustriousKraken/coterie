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

/// URL/stored-path prefix for the public root, served by
/// `GET /uploads/:filename`.
const PUBLIC_PREFIX: &str = "uploads/";

/// Stored-path prefix for the private root
/// (`ServerConfig::private_uploads_path()`). It is NOT a URL — nothing is
/// mounted on it; only authorization-gated handlers read from that root.
/// Stored paths carry it so a path alone says which root it lives in.
const PRIVATE_PREFIX: &str = "private-uploads/";

/// Strip either upload prefix off a stored path and return the bare
/// filename, or `None` if the path isn't one of ours or is doing path
/// trickery. Callers pick the root to join it onto.
pub fn upload_filename(stored_path: &str) -> Option<&str> {
    let filename = stored_path
        .strip_prefix(PUBLIC_PREFIX)
        .or_else(|| stored_path.strip_prefix(PRIVATE_PREFIX))?;
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
    {
        return None;
    }
    Some(filename)
}

/// Save an uploaded document (PDF only) into the **private** uploads root,
/// reusing the generated-name + size-cap behavior of
/// [`save_uploaded_file`]. The content is confirmed by magic-byte sniff;
/// the client-supplied filename/extension never influences acceptance or
/// the storage path. Returns the relative path
/// (e.g. `private-uploads/abc123.pdf`) — the prefix records which root the
/// file lives in, and the public route is not mounted on that root.
pub async fn save_uploaded_document(uploads_dir: &str, data: &[u8]) -> Result<String> {
    if data.len() > MAX_FILE_SIZE {
        return Err(AppError::Validation(
            "File too large (max 10 MB)".to_string(),
        ));
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

    Ok(format!("{}{}", PRIVATE_PREFIX, new_filename))
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
    Ok(format!("{}{}", PUBLIC_PREFIX, new_filename))
}

/// Delete an uploaded file by its stored path (e.g. "uploads/abc123.jpg"
/// or "private-uploads/abc123.pdf"). No-op if the path doesn't match our
/// upload convention, the filename is empty, or the file simply doesn't
/// exist.
///
/// `uploads_dir` is the configured filesystem root the caller owns
/// (`ServerConfig::uploads_path()` or `private_uploads_path()`); we join
/// the filename onto it to find the actual file. Path traversal is
/// blocked — any "." or ".." segments or absolute paths make this no-op.
pub async fn delete_uploaded_file(uploads_dir: &str, url_path: &str) -> Result<()> {
    let filename = match upload_filename(url_path) {
        Some(f) => f,
        None => return Ok(()),
    };

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

/// Move every stored submission attachment out of the public root and
/// into the private one, rewriting `submissions.attachment_path` to the
/// private prefix. Run once at startup, before the server accepts traffic.
///
/// Order is load-bearing: move the file first, rewrite the row second, so
/// an interrupted run leaves rows pointing at files that still exist.
/// Idempotent — a second run finds no `uploads/%` attachment paths and
/// does nothing.
///
/// Unreferenced files in the public root are left alone. Telling an
/// already-orphaned attachment apart from a legitimately public upload
/// after the fact is guesswork, so we only report the count and let the
/// operator decide whether this deployment warrants an audit.
pub async fn migrate_attachments_to_private_root(
    db_pool: &SqlitePool,
    public_dir: &str,
    private_dir: &str,
) -> Result<()> {
    fs::create_dir_all(private_dir).await.map_err(|e| {
        AppError::Internal(format!("Failed to create private uploads directory: {}", e))
    })?;

    let stale: Vec<(String,)> = sqlx::query_as(
        "SELECT attachment_path FROM submissions WHERE attachment_path LIKE 'uploads/%'",
    )
    .fetch_all(db_pool)
    .await?;

    let mut moved = 0usize;
    for (stored,) in &stale {
        let Some(filename) = upload_filename(stored) else {
            continue;
        };
        let from = PathBuf::from(public_dir).join(filename);
        let to = PathBuf::from(private_dir).join(filename);
        if from.exists() && !move_file(&from, &to).await {
            // Leave the row alone: it still names a file that exists.
            continue;
        }
        sqlx::query("UPDATE submissions SET attachment_path = ? WHERE attachment_path = ?")
            .bind(format!("{}{}", PRIVATE_PREFIX, filename))
            .bind(stored)
            .execute(db_pool)
            .await?;
        moved += 1;
    }
    if moved > 0 {
        tracing::info!(
            "Moved {} submission attachment(s) to {}",
            moved,
            private_dir
        );
    }

    report_unreferenced(db_pool, public_dir).await;
    Ok(())
}

/// Rename, falling back to copy+remove when the two roots are on
/// different filesystems (`uploads_dir` can be overridden onto its own
/// volume). Returns whether the file ended up at `to`.
async fn move_file(from: &std::path::Path, to: &std::path::Path) -> bool {
    if fs::rename(from, to).await.is_ok() {
        return true;
    }
    if let Err(e) = fs::copy(from, to).await {
        tracing::warn!("Failed to move {}: {}", from.display(), e);
        return false;
    }
    if let Err(e) = fs::remove_file(from).await {
        // The copy landed, so the row can safely be rewritten; the stale
        // public copy is the operator's to sweep.
        tracing::warn!(
            "Copied {} to the private root but could not remove the public copy: {}",
            from.display(),
            e
        );
    }
    true
}

/// Log how many files in the public root no row names, so the operator
/// learns whether a manual audit is warranted. Counting only — see the
/// doc comment on [`migrate_attachments_to_private_root`].
async fn report_unreferenced(db_pool: &SqlitePool, public_dir: &str) {
    let referenced: Vec<(String,)> = match sqlx::query_as(
        r#"
        SELECT image_url FROM events WHERE image_url IS NOT NULL
        UNION
        SELECT image_url FROM announcements WHERE image_url IS NOT NULL
        UNION
        SELECT attachment_path FROM submissions WHERE attachment_path IS NOT NULL
        "#,
    )
    .fetch_all(db_pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Could not audit the uploads directory: {}", e);
            return;
        }
    };
    let known: std::collections::HashSet<&str> = referenced
        .iter()
        .filter_map(|(p,)| upload_filename(p))
        .collect();

    let Ok(mut entries) = fs::read_dir(public_dir).await else {
        return;
    };
    let mut unreferenced = 0usize;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| !known.contains(n))
        {
            unreferenced += 1;
        }
    }
    if unreferenced > 0 {
        tracing::info!(
            "{} file(s) in {} are named by no row and are left in place. Anonymous callers \
             can no longer fetch them (nothing affirms they are public), but any \
             authenticated member still can — so an attachment orphaned before this \
             release is worth a manual audit.",
            unreferenced,
            public_dir
        );
    }
}

/// Whether `filename` is affirmatively known to be a **public** upload:
/// the image of a `Public` event or of a public announcement.
///
/// Phrased positively on purpose. The inverse ("is this private?") makes
/// disclosure the default for every file the query fails to recognise —
/// and a query stops recognising a file for reasons that have nothing to
/// do with intent: the row was deleted, cascaded away with its owner,
/// repointed at a different file, or not committed yet. Both phrasings
/// cost the same query; one fails toward a broken image and the other
/// toward publication.
///
/// A DB error also answers "not public", so an outage denies rather than
/// publishes.
///
/// This is the complete allow-list of public-root writers: event images
/// (`admin/events/single.rs`) and announcement images
/// (`admin/announcements.rs`). A new upload category must be registered
/// here — forgetting shows up as a broken asset, not as a silent leak.
async fn is_public_image(db_pool: &SqlitePool, filename: &str) -> bool {
    let full_path = format!("{}{}", PUBLIC_PREFIX, filename);

    let hit: Option<(i32,)> = sqlx::query_as(
        r#"
        SELECT 1 FROM events WHERE image_url = ? AND visibility = 'Public'
        UNION ALL
        SELECT 1 FROM announcements WHERE image_url = ? AND is_public = 1
        LIMIT 1
        "#,
    )
    .bind(&full_path)
    .bind(&full_path)
    .fetch_optional(db_pool)
    .await
    .ok()
    .flatten();

    hit.is_some()
}

/// Serve uploaded files from the PUBLIC root only.
///
/// Submission attachments are not reachable here at all — they live in
/// the private root, which this handler never joins onto — so there is no
/// attachment allow-or-deny decision left to make and no `submissions`
/// lookup. Anything the public-image allow-list cannot affirm requires an
/// authenticated session.
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

    // Allow-list: serve outright only what the database affirms is public.
    // Everything else — a members-only image, an image whose row is gone,
    // a file nothing claims — requires a session.
    if !is_public_image(&db_pool, &filename).await {
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

    // The PUBLIC root, always — the private root is never joined here.
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
