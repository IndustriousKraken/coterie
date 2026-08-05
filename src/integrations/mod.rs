use crate::domain::{Announcement, Event, EventVisibility, Member};
use crate::error::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

pub mod admin_alert_email;
pub mod discord;
pub mod discord_client;
pub mod public_site;
pub mod unifi;

#[derive(Debug, Clone)]
pub enum IntegrationEvent {
    MemberActivated(Member),
    MemberExpired(Member),
    MemberUpdated {
        old: Member,
        new: Member,
    },
    /// An event was **created** and is not `AdminOnly`. Visibility
    /// decides which Discord channel (if any) the integration routes
    /// this to — AdminOnly events go to the admin-alerts channel,
    /// others to the events channel.
    ///
    /// Do NOT read this as "this event is now public": it does not fire
    /// when an existing event's visibility changes, and it does fire for
    /// members-only events. A consumer that needs to know whether
    /// something is publicly visible reads that from the carried
    /// visibility, and learns of later changes from [`Self::EventUpdated`].
    EventPublished(Event),
    /// An existing event changed in a way a public consumer could
    /// observe — a content edit, a reschedule, or a visibility change in
    /// either direction. Carries the **post-update** state only.
    ///
    /// One variant covers retraction, late publication, and ordinary
    /// edits alike: visibility is a field on the state already carried,
    /// so `Unpublished` / `Republished` / `Rescheduled` variants would
    /// multiply the enum without telling a consumer anything the current
    /// state does not.
    EventUpdated(Event),
    /// An event was deleted. Carries its last state.
    EventDeleted(Event),
    /// An announcement transitioned from draft to published — either
    /// via `publish_now` on create or the dedicated publish action.
    AnnouncementPublished(Announcement),
    /// An existing announcement changed observably, including being
    /// unpublished. Post-update state only, for the same reason as
    /// [`Self::EventUpdated`].
    AnnouncementUpdated(Announcement),
    /// An announcement was deleted. Carries its last state.
    AnnouncementDeleted(Announcement),
    /// Operational notification for admins. Free-form subject/body so
    /// any subsystem can dispatch one without coordinating with the
    /// integration layer's enums.
    AdminAlert {
        subject: String,
        body: String,
    },
}

// ---------------------------------------------------------------------
// What may leave, and when.
//
// These live next to the enum on purpose: they are the contract the
// content variants carry, not a detail of any one consumer. Reducing a
// payload at the source makes disclosure unreachable; asking every
// present and future consumer to decline to republish private content
// makes it merely discouraged, and the failure is silent.
// ---------------------------------------------------------------------

/// Whether an announcement is on `/public/announcements` right now.
pub fn announcement_is_public(announcement: &Announcement) -> bool {
    announcement.is_public && announcement.published_at.is_some()
}

/// Reduce an event to no more than its own visibility already
/// discloses. `Public` keeps what the public feed publishes for it,
/// `MembersOnly` is cut down to exactly what `/public/events` returns
/// (via the same sanitizer the feed uses), and `AdminOnly` keeps
/// identity and visibility with no content at all.
pub fn redact_event_for_dispatch(mut event: Event) -> Event {
    match event.visibility {
        EventVisibility::Public => {}
        EventVisibility::MembersOnly => event.sanitize_members_only(),
        EventVisibility::AdminOnly => {
            // Nothing about an admin-only event is disclosed anywhere,
            // so a consumer gets what it needs to drop the item and
            // nothing else. The timestamps stay because the struct
            // requires values; the content does not.
            event.title = String::new();
            event.description = String::new();
            event.location = None;
            event.image_url = None;
            event.guest_registration_enabled = false;
        }
    }
    event
}

/// Announcement counterpart of [`redact_event_for_dispatch`]: an
/// announcement the public API does not serve carries no content.
pub fn redact_announcement_for_dispatch(mut announcement: Announcement) -> Announcement {
    if !announcement_is_public(&announcement) {
        announcement.title = String::new();
        announcement.content = String::new();
        announcement.image_url = None;
    }
    announcement
}

/// Fingerprint of everything a public consumer could observe about an
/// event: the redacted value minus the fields `/public/events` omits.
///
/// Stated as a subtraction rather than a list of observable fields so a
/// field added to `Event` later counts as observable until someone says
/// otherwise — an extra dispatch, never a missed one.
fn event_public_facet(event: &Event) -> String {
    facet(
        &redact_event_for_dispatch(event.clone()),
        &[
            "created_by",
            "created_at",
            "updated_at",
            "event_type_id",
            "series_id",
            "occurrence_index",
            "member_price_cents",
        ],
    )
}

/// Announcement counterpart of [`event_public_facet`]. `is_public` is
/// subtracted because its effect is already carried by the redaction —
/// flipping it on a draft changes nothing anyone can see.
fn announcement_public_facet(announcement: &Announcement) -> String {
    facet(
        &redact_announcement_for_dispatch(announcement.clone()),
        &[
            "created_by",
            "created_at",
            "updated_at",
            "announcement_type_id",
            "is_public",
            "scheduled_publish_at",
            "scheduled_publish_timezone",
        ],
    )
}

fn facet<T: serde::Serialize>(value: &T, omitted: &[&str]) -> String {
    let mut json = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
    if let Some(map) = json.as_object_mut() {
        for key in omitted {
            map.remove(*key);
        }
    }
    json.to_string()
}

/// Whether an edit changed anything a public consumer could observe.
/// An item that was `AdminOnly` before and after is invisible either
/// way, and an edit confined to fields the public projection omits
/// changes no public output — neither is worth dispatching.
pub fn event_change_is_public(before: &Event, after: &Event) -> bool {
    if before.visibility == EventVisibility::AdminOnly
        && after.visibility == EventVisibility::AdminOnly
    {
        return false;
    }
    event_public_facet(before) != event_public_facet(after)
}

/// Announcement counterpart of [`event_change_is_public`].
pub fn announcement_change_is_public(before: &Announcement, after: &Announcement) -> bool {
    if !announcement_is_public(before) && !announcement_is_public(after) {
        return false;
    }
    announcement_public_facet(before) != announcement_public_facet(after)
}

/// Which path carries a change to the configured companion public site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Rides the integration fan-out. Losing one is acceptable: the
    /// receiver's own reconciling sweep catches it, and the cost of
    /// being briefly stale about an addition is an addition arriving
    /// late.
    Bus,
    /// Delivered synchronously from the admin action so its outcome can
    /// be reported to the person who just clicked. A withdrawal that
    /// fails to arrive leaves content public with nobody aware, and a
    /// best-effort channel cannot carry a control whose failure is a
    /// disclosure.
    Synchronous,
    /// Nothing for the public site to hear about.
    None,
}

/// The single rule deciding [`Delivery`], with exactly two callers: the
/// dispatch site (which sends the synchronous half) and the notifier's
/// `handle_event` (which sends the bus half). A second copy of this rule
/// would either double-send or drop notifications silently, and the
/// second failure is invisible.
pub fn delivery_for(event: &IntegrationEvent) -> Delivery {
    match event {
        // Every deletion is a withdrawal.
        IntegrationEvent::EventDeleted(_) | IntegrationEvent::AnnouncementDeleted(_) => {
            Delivery::Synchronous
        }
        // An update landing short of fully public is treated as a
        // withdrawal. That is deliberately a superset — a members-only
        // reschedule is not a retraction — because the cost of the wider
        // rule is an admin being told whether the public site kept up,
        // which is never the wrong thing to tell them.
        IntegrationEvent::EventUpdated(e) if e.visibility != EventVisibility::Public => {
            Delivery::Synchronous
        }
        IntegrationEvent::AnnouncementUpdated(a) if !announcement_is_public(a) => {
            Delivery::Synchronous
        }
        // Creations and ordinary public-to-public edits.
        IntegrationEvent::EventPublished(_)
        | IntegrationEvent::EventUpdated(_)
        | IntegrationEvent::AnnouncementUpdated(_) => Delivery::Bus,
        // A published-but-not-public announcement is portal-only; the
        // public site has nothing to fetch for it.
        IntegrationEvent::AnnouncementPublished(a) => {
            if announcement_is_public(a) {
                Delivery::Bus
            } else {
                Delivery::None
            }
        }
        // Member and operational traffic is not public content.
        IntegrationEvent::MemberActivated(_)
        | IntegrationEvent::MemberExpired(_)
        | IntegrationEvent::MemberUpdated { .. }
        | IntegrationEvent::AdminAlert { .. } => Delivery::None,
    }
}

#[async_trait]
pub trait Integration: Send + Sync {
    fn name(&self) -> &str;
    fn is_enabled(&self) -> bool;
    async fn health_check(&self) -> Result<()>;
    async fn handle_event(&self, event: &IntegrationEvent) -> Result<()>;
}

pub struct IntegrationManager {
    integrations: RwLock<Vec<Arc<dyn Integration>>>,
}

impl IntegrationManager {
    pub fn new() -> Self {
        Self {
            integrations: RwLock::new(Vec::new()),
        }
    }

    pub async fn register(&self, integration: Arc<dyn Integration>) {
        if integration.is_enabled() {
            let mut integrations = self.integrations.write().await;
            integrations.push(integration);
            tracing::info!(
                "Registered integration: {}",
                integrations.last().unwrap().name()
            );
        }
    }

    pub async fn handle_event(&self, event: IntegrationEvent) {
        let integrations = self.integrations.read().await;

        for integration in integrations.iter() {
            if !integration.is_enabled() {
                continue;
            }

            match integration.handle_event(&event).await {
                Ok(_) => {
                    tracing::debug!(
                        "Integration {} handled event successfully",
                        integration.name()
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Integration {} failed to handle event: {:?}",
                        integration.name(),
                        e
                    );
                    // Continue processing other integrations even if one fails
                }
            }
        }
    }

    pub async fn health_check_all(&self) -> Vec<(String, Result<()>)> {
        let integrations = self.integrations.read().await;
        let mut results = Vec::new();

        for integration in integrations.iter() {
            let name = integration.name().to_string();
            let result = integration.health_check().await;
            results.push((name, result));
        }

        results
    }
}

// Base implementation for common integration functionality
pub struct BaseIntegration {
    pub name: String,
    pub enabled: bool,
}

impl BaseIntegration {
    pub fn new(name: impl Into<String>, enabled: bool) -> Self {
        Self {
            name: name.into(),
            enabled,
        }
    }
}
