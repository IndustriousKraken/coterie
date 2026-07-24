//! Regression test for the generic admin settings page's boolean
//! toggle. The original markup gave BOTH the visual checkbox and the
//! hidden state input `name="setting_value"`, so a checked box POSTed
//! the field twice and the Form extractor rejected the save (duplicate
//! field) — no boolean on the page could ever be changed. The checkbox
//! must stay unnamed; the hidden input is the single submitted value.

use askama::Template;
use coterie::web::portal::admin::settings::{
    AdminSettingsTemplate, SettingInfo, SettingsCategoryInfo,
};
use coterie::web::templates::BaseContext;

fn render_with_boolean_setting() -> String {
    AdminSettingsTemplate {
        base: BaseContext::default(),
        categories: vec![SettingsCategoryInfo {
            name: "membership".to_string(),
            display_name: "Membership".to_string(),
            description: "Membership settings".to_string(),
            settings: vec![SettingInfo {
                key: "membership.example_flag".to_string(),
                display_name: "Example Flag".to_string(),
                value: "false".to_string(),
                value_type: "boolean".to_string(),
                description: Some("A boolean setting".to_string()),
                is_sensitive: false,
                is_timezone: false,
                timezone_options: vec![],
                is_signup_mode: false,
                signup_mode_options: vec![],
            }],
        }],
        success_message: None,
        error_message: None,
    }
    .render()
    .expect("settings template renders")
}

#[test]
fn boolean_setting_submits_exactly_one_setting_value_field() {
    let html = render_with_boolean_setting();

    let occurrences = html.matches(r#"name="setting_value""#).count();
    assert_eq!(
        occurrences, 1,
        "a boolean setting block must contain exactly one input named \
         setting_value (the hidden state input); a second one (e.g. a \
         named checkbox) makes the POST a duplicate-field error"
    );
    assert!(
        html.contains(r#"type="hidden" name="setting_value""#),
        "the single setting_value input must be the hidden one"
    );
}

#[test]
fn toggle_syncs_the_hidden_input_not_itself() {
    let html = render_with_boolean_setting();
    assert!(
        html.contains("input[type=hidden][name=setting_value]"),
        "the checkbox onchange must target the hidden input explicitly"
    );
}
