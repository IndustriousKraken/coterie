//! Shared validators used by every configurable-type service. All three
//! services (event-type, announcement-type, membership-type) call
//! `validate_hex_color_for_request`; the basic-type service additionally uses
//! `check_unique_slug_for_basic` and `check_delete_unused_for_basic`.

use uuid::Uuid;

use crate::{
    domain::{validate_hex_color, BasicTypeKind},
    error::{AppError, Result},
    repository::BasicTypeRepository,
};

/// Reject malformed hex colors. `None` is allowed (color is optional).
pub(crate) fn validate_hex_color_for_request(color: Option<&str>) -> Result<()> {
    if let Some(c) = color {
        if !validate_hex_color(c) {
            return Err(AppError::BadRequest(format!(
                "Invalid color format: {}. Expected hex color like #FF0000",
                c
            )));
        }
    }
    Ok(())
}

/// Reject a duplicate slug. The `display_singular` argument is "event type"
/// or "announcement type" — used purely for the user-visible error message.
pub(crate) async fn check_unique_slug_for_basic(
    repo: &dyn BasicTypeRepository,
    kind: BasicTypeKind,
    slug: &str,
    display_singular: &str,
) -> Result<()> {
    if repo.find_by_slug(kind, slug).await?.is_some() {
        // Capitalize the first letter of the display name for the error.
        let mut first = display_singular.chars();
        let capitalized = match first.next() {
            Some(c) => c.to_uppercase().collect::<String>() + first.as_str(),
            None => String::new(),
        };
        return Err(AppError::Conflict(format!(
            "{} with slug '{}' already exists",
            capitalized, slug
        )));
    }
    Ok(())
}

/// Reject deletion when the type is still referenced. Produces an error
/// message of the form: "Cannot delete <display>: N <plural> still use this
/// type. Deactivate instead."
pub(crate) async fn check_delete_unused_for_basic(
    repo: &dyn BasicTypeRepository,
    kind: BasicTypeKind,
    id: Uuid,
) -> Result<()> {
    let usage_count = repo.count_usage(kind, id).await?;
    if usage_count > 0 {
        return Err(AppError::Conflict(format!(
            "Cannot delete {}: {} {} still use this type. Deactivate instead.",
            kind.display_name(),
            usage_count,
            kind.usage_noun_plural(),
        )));
    }
    Ok(())
}
