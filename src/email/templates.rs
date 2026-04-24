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
}

#[derive(Template)]
#[template(path = "emails/welcome.txt")]
pub struct WelcomeText<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub portal_url: &'a str,
}

#[derive(Template)]
#[template(path = "emails/reminder.html")]
pub struct ReminderHtml<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub due_date: &'a str,
    pub days_remaining: i64,
    pub pay_url: &'a str,
}

#[derive(Template)]
#[template(path = "emails/reminder.txt")]
pub struct ReminderText<'a> {
    pub full_name: &'a str,
    pub org_name: &'a str,
    pub due_date: &'a str,
    pub days_remaining: i64,
    pub pay_url: &'a str,
}
