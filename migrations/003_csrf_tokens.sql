-- CSRF tokens table
-- Tokens are tied to sessions and validated on state-changing requests
CREATE TABLE IF NOT EXISTS csrf_tokens (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_csrf_tokens_session ON csrf_tokens(session_id);
