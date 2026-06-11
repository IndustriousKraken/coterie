-- Track Stripe webhook event IDs we've already processed, so retries
-- (network flakes, Stripe redelivery, user-replayed webhooks) are
-- handled idempotently instead of double-extending dues or re-charging.

CREATE TABLE IF NOT EXISTS processed_stripe_events (
    event_id     TEXT PRIMARY KEY,
    event_type   TEXT NOT NULL,
    processed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_processed_stripe_events_processed_at
    ON processed_stripe_events(processed_at);
