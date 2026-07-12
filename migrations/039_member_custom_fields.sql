-- Org-defined member fields (see the member-custom-fields change).
-- Definitions are admin-managed; values hang off (member, field) with
-- cascades both ways: deleting a member or a definition removes the
-- stored values. field_key is the stable identifier forms post under
-- (display name can be renamed freely).
CREATE TABLE member_field_definitions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    field_key TEXT NOT NULL UNIQUE,
    field_type TEXT NOT NULL DEFAULT 'text' CHECK (field_type IN ('text', 'url')),
    member_editable INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE member_field_values (
    member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    field_id TEXT NOT NULL REFERENCES member_field_definitions(id) ON DELETE CASCADE,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (member_id, field_id)
);

CREATE INDEX idx_member_field_values_field ON member_field_values(field_id);
