# Tasks

## 1. Derive cors_origins from the marketing domain

- [x] 1.1 In `deploy/coterie-provision/src/env_template.rs`, add a
  `cors_origins: Option<String>` to `EnvConfig` populated from the
  marketing domain as `https://<domain>,https://www.<domain>`, skipping the
  `www.` variant when the domain already begins with `www.`. Leave it
  `None` when no marketing domain was supplied.
- [x] 1.2 Add a rewrite for the `COTERIE__SERVER__CORS_ORIGINS` line
  (mirroring the commented-line handling the integration blocks use via
  `match_commented_key`): uncomment and set it when `cors_origins` is
  `Some`, leave it commented when `None`.
- [x] 1.3 In `deploy/coterie-provision/src/install.rs`, pass the resolved
  marketing domain into `EnvConfig` when rendering `.env`.

## 2. Tests

- [x] 2.1 Extend the `.env` golden snapshot (or add an `env_template` unit
  test) asserting a run with marketing domain `example.org` renders
  `COTERIE__SERVER__CORS_ORIGINS=https://example.org,https://www.example.org`
  uncommented.
- [x] 2.2 Add a case asserting a run with NO marketing domain leaves
  `COTERIE__SERVER__CORS_ORIGINS` unset/commented (same-origin default
  preserved).
- [x] 2.3 Add a case for a marketing domain already prefixed with `www.`
  (e.g. `www.example.org`) asserting the origin is not double-prefixed.
