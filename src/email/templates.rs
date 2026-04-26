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

#[derive(Template)]
#[template(path = "emails/card_declined.html")]
pub struct CardDeclinedHtml<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    /// Formatted amount (e.g. "$50.00") if known. Sometimes Stripe
    /// invoices arrive without an amount field on the failed-payment
    /// event — render the message without it in that case.
    pub amount: Option<&'a str>,
    pub portal_url: &'a str,
    /// Formatted "your access is good through {date}" string.
    pub dues_until: &'a str,
    /// True when this is Stripe's last retry; we soften "we'll try
    /// again" to "this was the last attempt."
    pub is_final: bool,
}

#[derive(Template)]
#[template(path = "emails/card_declined.txt")]
pub struct CardDeclinedText<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub amount: Option<&'a str>,
    pub portal_url: &'a str,
    pub dues_until: &'a str,
    pub is_final: bool,
}

#[derive(Template)]
#[template(path = "emails/subscription_cancelled.html")]
pub struct SubscriptionCancelledHtml<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub dues_until: &'a str,
    pub portal_url: &'a str,
}

#[derive(Template)]
#[template(path = "emails/subscription_cancelled.txt")]
pub struct SubscriptionCancelledText<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub dues_until: &'a str,
    pub portal_url: &'a str,
}

#[derive(Template)]
#[template(path = "emails/admin_alert.html")]
pub struct AdminAlertHtml<'a> {
    pub org_name: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
}

#[derive(Template)]
#[template(path = "emails/admin_alert.txt")]
pub struct AdminAlertText<'a> {
    pub org_name: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
}
