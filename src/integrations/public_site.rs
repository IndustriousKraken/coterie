//! Change notifications for a companion public website.
//!
//! An organization's public site renders Coterie's public events and
//! announcements. Without this it can only learn of a change by polling,
//! which puts a floor on how wrong it can be — and the cost of being
//! wrong is not symmetric. A stale addition is an annoyance; a stale
//! retraction is a disclosure.
//!
//! Two properties do the work here:
//!
//! 1. **The notification carries no item content** — kind, id, and what
//!    happened. A receiver reads content from the public API, which
//!    already applies the org's visibility rules. That makes disclosure
//!    unaskable rather than carefully avoided: with an identifier alone,
//!    a receiver cannot render anything it was not already entitled to
//!    fetch, whatever it does with the message.
//! 2. **Withdrawals are acknowledged, everything else is best-effort.**
//!    See [`crate::integrations::Delivery`].
//!
//! This does NOT replace the receiver's own reconciling sweep. Blind
//! push has no self-healing: one dropped notification and the site
//! drifts silently. What push buys is that the sweep can run hourly
//! instead of every few minutes.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

use crate::{
    error::Result,
    integrations::{delivery_for, Delivery, Integration, IntegrationEvent, IntegrationManager},
    service::settings_service::SettingsService,
};

type HmacSha256 = Hmac<Sha256>;

/// Every attempt is bounded by this. An unresponsive endpoint delays an
/// admin's response by at most this much rather than hanging it.
pub const NOTIFY_TIMEOUT: Duration = Duration::from_secs(5);

/// Header carrying `sha256=<hex>` over the exact request body.
pub const SIGNATURE_HEADER: &str = "X-Coterie-Signature";

pub const KIND_EVENT: &str = "event";
pub const KIND_ANNOUNCEMENT: &str = "announcement";
pub const ACTION_UPDATED: &str = "updated";
pub const ACTION_DELETED: &str = "deleted";

/// What the public-site notification managed to do, for reporting back
/// to the admin who triggered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicSiteOutcome {
    /// No endpoint configured, or this change rides the fan-out and has
    /// no synchronous result to report.
    NotAttempted,
    Sent,
    Failed(String),
}

impl PublicSiteOutcome {
    /// A sentence to append to the admin's response, or `None` when
    /// there is nothing to say. A silent failure here is the defect this
    /// capability exists to fix, so a failure always names the retry.
    pub fn admin_note(&self) -> Option<String> {
        match self {
            PublicSiteOutcome::NotAttempted => None,
            PublicSiteOutcome::Sent => Some("The public site was updated.".to_string()),
            PublicSiteOutcome::Failed(err) => Some(format!(
                "The public site was NOT updated ({}). Open this item and use \
                 \"Resend to public site\" to retry.",
                err
            )),
        }
    }

    /// [`Self::admin_note`] for an item that no longer exists, where the
    /// per-item resend control is not a retry the admin can reach. The
    /// item is deleted in Coterie either way — a failed notification
    /// never rolls the deletion back, because reverting would make the
    /// two systems more inconsistent rather than less.
    pub fn admin_note_deleted(&self) -> Option<String> {
        match self {
            PublicSiteOutcome::Failed(err) => Some(format!(
                "The public site was NOT told about the deletion ({}). It is deleted here \
                 regardless, and the public site will drop it at its next reconcile.",
                err
            )),
            other => other.admin_note(),
        }
    }

    /// True when a notification was actually attempted and succeeded.
    pub fn is_sent(&self) -> bool {
        matches!(self, PublicSiteOutcome::Sent)
    }

    /// True when a notification was attempted and did not get through —
    /// the only case an admin has to act on.
    pub fn is_failed(&self) -> bool {
        matches!(self, PublicSiteOutcome::Failed(_))
    }
}

/// `sha256=<hex>` of the HMAC-SHA256 of `body` under `secret`. Exposed
/// so a receiver implementation (and the tests) can verify against the
/// same code path that signs.
pub fn sign_body(secret: &str, body: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts a key of any length");
    mac.update(body.as_bytes());
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// Posts signed, content-free change notifications to the configured
/// companion site. Reads its config from the DB per call, so an admin
/// can point it at a new endpoint (or turn it off) without a restart.
pub struct PublicSiteNotifier {
    settings: Arc<SettingsService>,
    client: reqwest::Client,
}

impl PublicSiteNotifier {
    pub fn new(settings: Arc<SettingsService>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(NOTIFY_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self { settings, client }
    }

    /// Whether a companion site is configured at all. Drives the admin
    /// resend control's visibility — with no endpoint there is nothing
    /// to resend to, and a button that cannot work is worse than no
    /// button.
    pub async fn is_configured(&self) -> bool {
        self.endpoint().await.is_some()
    }

    /// The configured endpoint + secret, or `None` when the capability
    /// is inert. A secret that will not decrypt (`session_secret`
    /// rotated) reads as unconfigured rather than sending unsigned.
    async fn endpoint(&self) -> Option<(String, String)> {
        let cfg = self.settings.get_public_site_config().await.ok()?;
        let url = cfg.endpoint_url.trim().to_string();
        (!url.is_empty()).then_some((url, cfg.secret))
    }

    /// Post one notification and report what happened. Absent an
    /// endpoint this is completely inert: no request, no error, no log
    /// line — a deployment without a companion site sees nothing.
    pub async fn notify(&self, kind: &str, id: Uuid, action: &str) -> PublicSiteOutcome {
        let Some((url, secret)) = self.endpoint().await else {
            return PublicSiteOutcome::NotAttempted;
        };

        // Kind, id, what happened, and when. No title, no body, no
        // location, no image — see the module docs for why that is
        // structural rather than careful.
        let body = serde_json::json!({
            "kind": kind,
            "id": id.to_string(),
            "action": action,
            "sent_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();

        let request = self
            .client
            .post(&url)
            .header(SIGNATURE_HEADER, sign_body(&secret, &body))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);

        match request.send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!("public-site notify {} {} {}: ok", kind, id, action);
                PublicSiteOutcome::Sent
            }
            Ok(resp) => {
                let status = resp.status();
                tracing::warn!(
                    "public-site notify {} {} {} rejected: HTTP {}",
                    kind,
                    id,
                    action,
                    status
                );
                PublicSiteOutcome::Failed(format!("the site answered HTTP {}", status.as_u16()))
            }
            Err(e) => {
                // Deliberately not logging the URL's credentials or the
                // secret — reqwest errors carry the URL only.
                tracing::warn!(
                    "public-site notify {} {} {} failed: {}",
                    kind,
                    id,
                    action,
                    e
                );
                PublicSiteOutcome::Failed(if e.is_timeout() {
                    "the site did not respond in time".to_string()
                } else {
                    "the site could not be reached".to_string()
                })
            }
        }
    }

    /// Deliver `event` now, if it is one the bus must not carry. Returns
    /// [`PublicSiteOutcome::NotAttempted`] for everything else, so a
    /// caller can hand the result straight to the admin without
    /// re-deciding which path applied.
    pub async fn notify_withdrawal(&self, event: &IntegrationEvent) -> PublicSiteOutcome {
        match (delivery_for(event), describe(event)) {
            (Delivery::Synchronous, Some((kind, id, action))) => {
                self.notify(kind, id, action).await
            }
            _ => PublicSiteOutcome::NotAttempted,
        }
    }

    /// Resend an item's current state on demand. The retry path when a
    /// notification failed, and the recovery path when the automatic one
    /// is broken for any reason — so it deliberately depends on no other
    /// part of this capability working.
    pub async fn resend(&self, kind: &str, id: Uuid) -> PublicSiteOutcome {
        self.notify(kind, id, ACTION_UPDATED).await
    }
}

/// The notification triple for a content variant, or `None` when the
/// variant is not about public content.
fn describe(event: &IntegrationEvent) -> Option<(&'static str, Uuid, &'static str)> {
    match event {
        IntegrationEvent::EventPublished(e) | IntegrationEvent::EventUpdated(e) => {
            Some((KIND_EVENT, e.id, ACTION_UPDATED))
        }
        IntegrationEvent::EventDeleted(e) => Some((KIND_EVENT, e.id, ACTION_DELETED)),
        IntegrationEvent::AnnouncementPublished(a) | IntegrationEvent::AnnouncementUpdated(a) => {
            Some((KIND_ANNOUNCEMENT, a.id, ACTION_UPDATED))
        }
        IntegrationEvent::AnnouncementDeleted(a) => Some((KIND_ANNOUNCEMENT, a.id, ACTION_DELETED)),
        _ => None,
    }
}

/// Notify the public site about `event` and then fan it out.
///
/// The synchronous half runs FIRST so the answer an admin is waiting on
/// is not queued behind Discord's latency. The returned outcome is
/// [`PublicSiteOutcome::NotAttempted`] for everything that rides the
/// bus, which is most traffic.
///
/// A future reader will be tempted to "fix" the asymmetry by moving the
/// withdrawal onto the bus. That would delete the guarantee:
/// `integration-events` specifies that consumers do not block the
/// originating call and that failures are logged rather than surfaced —
/// correct for a missed Discord post, which a human can repost, and
/// wrong for a retraction, where the failure is content staying public
/// with nobody aware. That requirement stays exactly as it is for the
/// traffic it governs; withdrawal is simply not that traffic.
pub async fn announce(
    manager: &IntegrationManager,
    notifier: &PublicSiteNotifier,
    event: IntegrationEvent,
) -> PublicSiteOutcome {
    let outcome = notifier.notify_withdrawal(&event).await;
    manager.handle_event(event).await;
    outcome
}

#[async_trait]
impl Integration for PublicSiteNotifier {
    fn name(&self) -> &str {
        "PublicSite"
    }

    fn is_enabled(&self) -> bool {
        // Always registered; configuration is re-read per event so an
        // admin can point it somewhere new without a restart.
        true
    }

    async fn health_check(&self) -> Result<()> {
        // Unconfigured is "intentionally off", not an error, and we
        // don't poke someone else's website on boot.
        Ok(())
    }

    async fn handle_event(&self, event: &IntegrationEvent) -> Result<()> {
        if delivery_for(event) != Delivery::Bus {
            // Either not public content, or a withdrawal the admin
            // action already delivered and reported on.
            return Ok(());
        }
        if let Some((kind, id, action)) = describe(event) {
            self.notify(kind, id, action).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_verifies_and_detects_tampering() {
        let body = r#"{"kind":"event","id":"abc","action":"deleted"}"#;
        let signature = sign_body("shared-secret", body);

        assert_eq!(signature, sign_body("shared-secret", body));
        assert_ne!(signature, sign_body("other-secret", body));
        assert_ne!(
            signature,
            sign_body("shared-secret", &body.replace("deleted", "updated")),
        );
        assert!(signature.starts_with("sha256="));
    }

    #[test]
    fn failure_note_names_the_retry_and_success_is_plain() {
        assert_eq!(PublicSiteOutcome::NotAttempted.admin_note(), None);
        assert_eq!(
            PublicSiteOutcome::Sent.admin_note().unwrap(),
            "The public site was updated."
        );
        let note = PublicSiteOutcome::Failed("boom".to_string())
            .admin_note()
            .unwrap();
        assert!(note.contains("NOT updated"), "{note}");
        assert!(note.contains("Resend to public site"), "{note}");
    }
}
