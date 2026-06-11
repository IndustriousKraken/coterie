## ADDED Requirements

### Requirement: Same-origin by default; explicit allowlist via setting

The CORS layer SHALL default to same-origin (no `Access-Control-Allow-Origin` header). Cross-origin access SHALL be enabled only by setting the `cors_origins` configuration to a comma-separated list of allowed origins.

#### Scenario: No setting means no cross-origin access

- **WHEN** `cors_origins` is unset or empty
- **THEN** the CORS layer SHALL not advertise any allowed origins; cross-origin browsers SHALL be blocked by the same-origin policy

#### Scenario: Configured allowlist permits listed origins

- **WHEN** `cors_origins` is set to `https://example.org,https://www.example.org`
- **THEN** the CORS layer SHALL allow those exact origins and only those

### Requirement: Allowed methods, headers, and credentials are fixed

The CORS layer SHALL allow methods GET, POST, PUT, DELETE, OPTIONS; allow headers Content-Type, Authorization, X-CSRF-Token; and allow credentials (cookies).

#### Scenario: Preflight succeeds for allowed origin

- **WHEN** an allowed origin sends a preflight OPTIONS request with `Access-Control-Request-Headers: X-CSRF-Token`
- **THEN** the response SHALL allow the header

### Requirement: CORS gates /public/* cross-origin POSTs in lieu of CSRF

`/public/signup` and `/public/donate` SHALL be CSRF-exempt because they are called cross-origin from the marketing site. The CORS allowlist combined with rate limiting and bot challenge SHALL be the security model for these endpoints.

#### Scenario: Cross-origin signup from non-allowed origin is blocked

- **WHEN** a browser at a non-allowlisted origin attempts a cross-origin POST to `/public/signup`
- **THEN** the browser SHALL block the request due to the CORS policy
