-- Remove the legacy Discord rows seeded by 001_initial_schema. The real
-- Discord configuration lives under the `discord` category (migration 012)
-- and is edited on /portal/admin/settings/discord; these two rows have no
-- runtime reader but still render on the generic admin settings page,
-- presenting a phantom second place to "configure Discord". Mirrors the
-- cleanup migrations 029 (stripe) and 030 (unifi) did for their legacy rows.
DELETE FROM app_settings WHERE key IN (
    'integrations.discord.enabled',
    'integrations.discord.guild_name'
);
