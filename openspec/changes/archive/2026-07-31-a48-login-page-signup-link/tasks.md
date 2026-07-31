# Tasks

## 1. Setting

- [x] 1.1 Migration: insert `org.signup_url` into `app_settings` — category
  `organization`, type `string`, default `''`, not sensitive, described as the
  public signup page members are sent to from the login screen.
- [x] 1.2 Confirm it renders as a normal text field on the admin settings page via
  the existing generic form; no special-casing needed.
- Confirmed by inspection: `organization` is already in `category_meta`
  (`src/web/portal/admin/settings.rs`), and a non-sensitive `string` setting
  falls through every special-case branch in `templates/admin/settings.html`
  to the plain `<input type="text">`. No code change needed.

## 2. Login page

- [x] 2.1 Supply `org.signup_url` to the login template from its handler.
- [x] 2.2 `templates/auth/login.html`: wrap the create-account block so it renders
  only when the value is non-empty, and point the `href` at it. Remove the
  hardcoded `/public/signup` target.
- [x] 2.3 Leave the "forgot password" link exactly as it is.

## 3. Tests

- [x] 3.1 Empty setting renders no create-account link — assert the rendered HTML
  contains neither the link text nor `/public/signup`.
- [x] 3.2 Configured setting renders a link to that URL.
- [x] 3.3 Assert no template in the repo references `/public/signup` as an `href`.
  This is the actual defect class; a grep-style assertion stops it returning.
