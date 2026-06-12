## ADDED Requirements

### Requirement: Announcement list preview tolerates multi-byte bodies

The admin announcements list (`GET /portal/admin/announcements`) SHALL build each row's content preview without panicking, regardless of the announcement body's length or UTF-8 content. The preview truncation SHALL cut on a UTF-8 character boundary, never on a raw byte index.

#### Scenario: Announcement body with a multi-byte character at the truncation boundary renders safely

- **GIVEN** an announcement whose body is longer than the preview limit and contains a multi-byte UTF-8 character (e.g. an emoji) straddling the limit
- **WHEN** an admin loads `GET /portal/admin/announcements`
- **THEN** the request SHALL complete without panicking and the preview SHALL be truncated on a character boundary with an ellipsis appended

#### Scenario: Short ASCII bodies are shown in full

- **WHEN** an announcement body is plain ASCII at or below the preview limit
- **THEN** the preview SHALL equal the full body with no ellipsis appended
