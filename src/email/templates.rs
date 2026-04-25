//! Askama templates for outbound emails. Each email type has two
//! templates — HTML and plain text — that get rendered into a
//! multipart/alternative message.

use askama::Template;

#[derive(Template)]
#[template(path = "emails/verify.html")]
pub struct VerifyHtml<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub verify_url: &'a str,
}

#[derive(Template)]
#[template(path = "emails/verify.txt")]
pub struct VerifyText<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub verify_url: &'a str,
}

#[derive(Template)]
#[template(path = "emails/reset.html")]
pub struct ResetHtml<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub reset_url: &'a str,
}

#[derive(Template)]
#[template(path = "emails/reset.txt")]
pub struct ResetText<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub reset_url: &'a str,
}

#[derive(Template)]
#[template(path = "emails/welcome.html")]
pub struct WelcomeHtml<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub portal_url: &'a str,
    /// If set, the welcome email includes a "join Discord" line. The
    /// admin configures this URL in Discord settings.
    pub discord_invite: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "emails/welcome.txt")]
pub struct WelcomeText<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub portal_url: &'a str,
    pub discord_invite: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "emails/reminder.html")]
pub struct ReminderHtml<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub due_date: &'a str,
    pub days_remaining: i64,
    pub pay_url: &'a str,
    /// When true, the reminder includes a "your saved card is invalid"
    /// callout. Used for members on auto-renew whose default card has
    /// expired (so the charge would otherwise fail silently).
    pub card_invalid: bool,
}

#[derive(Template)]
#[template(path = "emails/reminder.txt")]
pub struct ReminderText<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub due_date: &'a str,
    pub days_remaining: i64,
    pub pay_url: &'a str,
    pub card_invalid: bool,
}

#[derive(Template)]
#[template(path = "emails/renewal_notice.html")]
pub struct RenewalNoticeHtml<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub due_date: &'a str,
    pub days_remaining: i64,
    pub amount: &'a str,
    pub card_display: &'a str,
    pub portal_url: &'a str,
}

#[derive(Template)]
#[template(path = "emails/renewal_notice.txt")]
pub struct RenewalNoticeText<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub due_date: &'a str,
    pub days_remaining: i64,
    pub amount: &'a str,
    pub card_display: &'a str,
    pub portal_url: &'a str,
}
