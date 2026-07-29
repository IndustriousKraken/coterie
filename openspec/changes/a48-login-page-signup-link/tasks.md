# Tasks

## 1. Setting

- [ ] 1.1 Migration: insert `org.signup_url` into `app_settings` — category
  `organization`, type `string`, default `''`, not sensitive, described as the
  public signup page members are sent to from the login screen.
- [ ] 1.2 Confirm it renders as a normal text field on the admin settings page via
  the existing generic form; no special-casing needed.

## 2. Login page

- [ ] 2.1 Supply `org.signup_url` to the login template from its handler.
- [ ] 2.2 `templates/auth/login.html`: wrap the create-account block so it renders
  only when the value is non-empty, and point the `href` at it. Remove the
  hardcoded `/public/signup` target.
- [ ] 2.3 Leave the "forgot password" link exactly as it is.

## 3. Tests

- [ ] 3.1 Empty setting renders no create-account link — assert the rendered HTML
  contains neither the link text nor `/public/signup`.
- [ ] 3.2 Configured setting renders a link to that URL.
- [ ] 3.3 Assert no template in the repo references `/public/signup` as an `href`.
  This is the actual defect class; a grep-style assertion stops it returning.

## 4. Deployment note

- [ ] 4.1 For The Neon Temple, set the value to `https://theneontemple.com/join/`
  after deploy. Until then the link is simply absent, which is the intended
  upgrade behavior.
