-- Member proposal submissions (see the member-proposal-submissions
-- OpenSpec change). A member authors a proposal (talk/session/etc.) that
-- an admin later reviews; the whole risk is that low-trust member content
-- — including an uploaded PDF — is opened by a higher-privileged admin,
-- so the read/authz paths are the security surface, not this schema.
--
-- Off by default: `submissions.enabled` gates every route and the portal
-- entry point, so an org that doesn't opt in gets no added surface.
CREATE TABLE IF NOT EXISTS submissions (
    id TEXT PRIMARY KEY NOT NULL,
    -- Set from the session, NEVER from the request body.
    submitter_member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    -- `abstract` is a Rust reserved word; the column is `abstract_text`
    -- so the domain field and SQL column line up without raw identifiers.
    abstract_text TEXT NOT NULL DEFAULT '',
    visibility_requested TEXT NOT NULL DEFAULT 'members'
        CHECK(visibility_requested IN ('public', 'members')),
    -- Server-generated `uploads/<uuid>.pdf` relative path, or NULL.
    attachment_path TEXT,
    -- Local wall-clock (naive) paired with `timezone`, following the
    -- event-timezone convention — never a frozen instant. NULL when the
    -- member gave no preferred date.
    preferred_start DATETIME,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    duration_minutes INTEGER,
    status TEXT NOT NULL DEFAULT 'submitted'
        CHECK(status IN ('submitted', 'under_review', 'accepted', 'declined', 'withdrawn', 'scheduled')),
    reviewer_note TEXT,
    decided_by TEXT REFERENCES members(id),
    -- Set on promotion to a standard Event (traceability link).
    event_id TEXT REFERENCES events(id) ON DELETE SET NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_submissions_submitter ON submissions(submitter_member_id);
CREATE INDEX IF NOT EXISTS idx_submissions_status ON submissions(status);

-- Feature toggle, default off. Category `submissions` so it renders as a
-- boolean toggle on the admin settings page via the settings service.
INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('submissions.enabled', 'false', 'boolean', 'submissions',
     'Allow members to submit talk/session proposals for admin review (off = no submission routes or UI)', 0);
