# provisioning-wizard Specification

## ADDED Requirements

### Requirement: Wizard configures CORS from the marketing domain

When the operator supplies a marketing domain, the wizard SHALL populate `COTERIE__SERVER__CORS_ORIGINS` in the rendered `.env` with the HTTPS origin(s) served by the Caddy marketing vhost for that domain (the apex and its `www.` variant), so the marketing site's browser requests to the public API (`/public/*`) are permitted by the `cors-policy` layer. When no marketing domain is supplied, the wizard SHALL leave `cors_origins` unset, preserving the same-origin default.

#### Scenario: Marketing domain sets the CORS allowlist

- **WHEN** the wizard renders `.env` for an install whose marketing domain is `example.org`
- **THEN** the rendered `.env` SHALL set `COTERIE__SERVER__CORS_ORIGINS` to an uncommented value that includes `https://example.org` and the `https://www.example.org` origin the Caddy marketing vhost also serves, so a browser at either origin is allowed to read `/public/*`

#### Scenario: No marketing domain leaves the same-origin default

- **WHEN** the wizard renders `.env` for an install with no marketing domain supplied
- **THEN** the rendered `.env` SHALL NOT set `COTERIE__SERVER__CORS_ORIGINS` (it remains unset or commented), and cross-origin browser access remains blocked per the `cors-policy` default

#### Scenario: Marketing domain already prefixed with www is not double-prefixed

- **WHEN** the supplied marketing domain already begins with `www.` (for example `www.example.org`)
- **THEN** the rendered `COTERIE__SERVER__CORS_ORIGINS` SHALL contain `https://www.example.org` and SHALL NOT contain a double-prefixed `https://www.www.example.org` origin
