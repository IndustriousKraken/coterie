-- Real FK from payments → donation_campaigns. The previous campaign-
-- progress query LIKE-matched campaign_id against payment description,
-- but donate_api stored the campaign NAME in description, so totals
-- always returned 0. With this FK + payment_type='donation' set
-- explicitly, get_total_donated can do an honest WHERE.
--
-- Existing payments stay attributed-to-nothing (NULL) — there's no
-- reliable way to back-derive campaign from a description string,
-- and the alternative (guessing) would corrupt totals when admins
-- start using the new flow.

ALTER TABLE payments ADD COLUMN donation_campaign_id TEXT
    REFERENCES donation_campaigns(id);

CREATE INDEX idx_payments_donation_campaign
    ON payments(donation_campaign_id)
    WHERE donation_campaign_id IS NOT NULL;
