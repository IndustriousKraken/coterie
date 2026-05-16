//! Golden-snapshot tests for the member-context Askama templates.
//!
//! Each test renders one of the affected templates with a fixed fixture
//! (specific UUID, fixed dates, a representative status variant) and
//! asserts that the rendered HTML matches a committed `.golden.html`
//! file under `tests/snapshots/`.
//!
//! Workflow: if a golden file is missing, the test writes it and
//! passes. On subsequent runs the test re-renders with the same fixture
//! and asserts byte-equality with the committed golden. Any drift
//! (changed format string, retyped field rendering differently, etc.)
//! fails the test and surfaces in CI.

use askama::Template;
use chrono::TimeZone;
use coterie::{
    domain::MemberStatus,
    web::{
        portal::{
            MemberInfo,
            admin::members::{
                AdminMemberDetailInfo, AdminMemberDetailTemplate, AdminMemberInfo,
                AdminMembersTableTemplate, AdminMembersTemplate, AdminNewMemberTemplate,
                AdminSavedCardInfo, MembershipTypeOption,
            },
            dashboard::MemberDashboardTemplate,
            profile::ProfileTemplate,
            security::SecurityTemplate,
        },
        templates::BaseContext,
    },
};
use std::path::PathBuf;
use uuid::Uuid;

const FIXTURE_UUID: &str = "11111111-2222-3333-4444-555555555555";

fn fixture_uuid() -> Uuid {
    Uuid::parse_str(FIXTURE_UUID).unwrap()
}

fn fixture_joined() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc.with_ymd_and_hms(2025, 9, 12, 14, 30, 0).unwrap()
}

fn fixture_dues() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap()
}

fn fixture_base() -> BaseContext {
    BaseContext::default()
}

fn member_info(status: MemberStatus) -> MemberInfo {
    MemberInfo {
        id: fixture_uuid(),
        username: "jdoe".to_string(),
        full_name: "Jane Doe".to_string(),
        email: "jane@example.com".to_string(),
        status,
        membership_type: "Regular".to_string(),
        joined_at: fixture_joined(),
        dues_paid_until: Some(fixture_dues()),
    }
}

fn admin_member_info(status: MemberStatus) -> AdminMemberInfo {
    AdminMemberInfo {
        id: fixture_uuid(),
        email: "jane@example.com".to_string(),
        username: "jdoe".to_string(),
        full_name: "Jane Doe".to_string(),
        initials: "JD".to_string(),
        status,
        membership_type: "Regular".to_string(),
        joined_at: fixture_joined(),
        dues_paid_until: Some(fixture_dues()),
    }
}

fn admin_member_detail_info(status: MemberStatus) -> AdminMemberDetailInfo {
    AdminMemberDetailInfo {
        id: fixture_uuid(),
        email: "jane@example.com".to_string(),
        username: "jdoe".to_string(),
        full_name: "Jane Doe".to_string(),
        initials: "JD".to_string(),
        status,
        membership_type_id: "00000000-0000-0000-0000-000000000001".to_string(),
        membership_type_name: "Regular".to_string(),
        joined_at: fixture_joined(),
        dues_paid_until: Some(fixture_dues()),
        dues_expired: false,
        bypass_dues: false,
        email_verified: true,
        notes: String::new(),
        billing_mode: "manual".to_string(),
        stripe_customer_id: None,
        stripe_subscription_id: None,
        discord_id: String::new(),
        saved_cards: Vec::<AdminSavedCardInfo>::new(),
        created_at: "September 12, 2025".to_string(),
        updated_at: "September 12, 2025 at  2:30 PM".to_string(),
    }
}

fn type_options() -> Vec<MembershipTypeOption> {
    vec![MembershipTypeOption {
        id: "00000000-0000-0000-0000-000000000001".to_string(),
        slug: "regular".to_string(),
        name: "Regular".to_string(),
    }]
}

fn snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("snapshots")
}

fn assert_golden(name: &str, rendered: &str) {
    let path = snapshots_dir().join(format!("{name}.golden.html"));
    if !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, rendered).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path).expect("read golden");
    assert_eq!(
        rendered, expected,
        "rendered output drifted from golden at {}",
        path.display()
    );
}

fn render_dashboard(status: MemberStatus) -> String {
    let tmpl = MemberDashboardTemplate {
        base: fixture_base(),
        member: member_info(status),
    };
    tmpl.render().expect("render dashboard")
}

fn render_profile(status: MemberStatus) -> String {
    let tmpl = ProfileTemplate {
        base: fixture_base(),
        member: member_info(status),
    };
    tmpl.render().expect("render profile")
}

fn render_security(status: MemberStatus) -> String {
    let tmpl = SecurityTemplate {
        base: fixture_base(),
        member: member_info(status),
        totp_enabled: false,
        recovery_codes_remaining: 0,
        admin_must_enroll: false,
    };
    tmpl.render().expect("render security")
}

fn render_admin_members(status: MemberStatus) -> String {
    let tmpl = AdminMembersTemplate {
        base: fixture_base(),
        members: vec![admin_member_info(status)],
        total_members: 1,
        current_page: 1,
        per_page: 20,
        total_pages: 1,
        search_query: String::new(),
        status_filter: String::new(),
        type_filter: String::new(),
        type_options: type_options(),
        sort_field: "name".to_string(),
        sort_order: "asc".to_string(),
    };
    tmpl.render().expect("render admin members")
}

fn render_admin_members_table(status: MemberStatus) -> String {
    let tmpl = AdminMembersTableTemplate {
        members: vec![admin_member_info(status)],
        total_members: 1,
        current_page: 1,
        per_page: 20,
        total_pages: 1,
        search_query: String::new(),
        status_filter: String::new(),
        type_filter: String::new(),
        sort_field: "name".to_string(),
        sort_order: "asc".to_string(),
    };
    tmpl.render().expect("render admin members table")
}

fn render_admin_member_detail(status: MemberStatus) -> String {
    let tmpl = AdminMemberDetailTemplate {
        base: fixture_base(),
        member: admin_member_detail_info(status),
        type_options: type_options(),
    };
    tmpl.render().expect("render admin member detail")
}

fn render_admin_member_new() -> String {
    let tmpl = AdminNewMemberTemplate {
        base: fixture_base(),
        type_options: type_options(),
    };
    tmpl.render().expect("render admin member new")
}

fn all_statuses() -> [(&'static str, MemberStatus); 5] {
    [
        ("active", MemberStatus::Active),
        ("pending", MemberStatus::Pending),
        ("expired", MemberStatus::Expired),
        ("suspended", MemberStatus::Suspended),
        ("honorary", MemberStatus::Honorary),
    ]
}

#[test]
fn dashboard_template_snapshots_match() {
    for (label, status) in all_statuses() {
        assert_golden(&format!("dashboard_member__{label}"), &render_dashboard(status));
    }
}

#[test]
fn profile_template_snapshots_match() {
    for (label, status) in all_statuses() {
        assert_golden(&format!("portal_profile__{label}"), &render_profile(status));
    }
}

#[test]
fn security_template_snapshots_match() {
    for (label, status) in all_statuses() {
        assert_golden(&format!("portal_security__{label}"), &render_security(status));
    }
}

#[test]
fn admin_members_template_snapshots_match() {
    for (label, status) in all_statuses() {
        assert_golden(&format!("admin_members__{label}"), &render_admin_members(status));
    }
}

#[test]
fn admin_members_table_template_snapshots_match() {
    for (label, status) in all_statuses() {
        assert_golden(
            &format!("admin_members_table__{label}"),
            &render_admin_members_table(status),
        );
    }
}

#[test]
fn admin_member_detail_template_snapshots_match() {
    for (label, status) in all_statuses() {
        assert_golden(
            &format!("admin_member_detail__{label}"),
            &render_admin_member_detail(status),
        );
    }
}

#[test]
fn admin_member_new_template_snapshot_matches() {
    assert_golden("admin_member_new", &render_admin_member_new());
}
