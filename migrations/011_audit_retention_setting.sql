-- How many days to keep entries in `audit_logs`. A background task
-- prunes older entries hourly (along with expired sessions). Lowering
-- this reduces backup size; raising it preserves more history for
-- post-incident review.

INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('audit.retention_days', '365', 'number', 'audit',
     'Days to keep entries in the admin audit log before automatic deletion', 0);
