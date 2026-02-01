use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

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
