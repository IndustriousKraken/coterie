-- The login page advertised a "create a new account" link pointing at
-- /public/signup — a POST-only JSON API, so a browser following it got a
-- 405 and downloaded the error body as a file. Coterie hosts no
-- self-service signup page and should not grow one: account creation
-- lives on the org's own public site, which posts to /public/signup.
--
-- So the link's destination becomes org-configured. Empty by default:
-- a deployment that has not configured a signup page advertises none,
-- because no link is strictly better than a broken one.
--
-- org.website_url is deliberately NOT reused — it answers "where is the
-- org's site", and its stock value is the https://example.com
-- placeholder, which would just be a different dead link on a fresh
-- install.
INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('org.signup_url', '', 'string', 'organization',
     'Public signup page members are sent to from the login screen. Leave empty to show no create-account link.', 0);
