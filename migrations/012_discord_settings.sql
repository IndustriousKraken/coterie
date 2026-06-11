-- Discord integration: configuration moves from env vars to DB-backed
-- settings, matching the email pattern. The bot token is encrypted at
-- rest using SecretCrypto (key derived from session_secret).
--
-- Bot setup is a Discord-side task: the operator creates a bot
-- application in Discord's developer portal, invites it to their
-- guild with bot+roles+messages permissions, then pastes the token,
-- guild ID, role IDs, and channel IDs here.

INSERT INTO app_settings (key, value, value_type, category, description, is_sensitive) VALUES
    ('discord.enabled', 'false', 'boolean', 'discord',
     'Enable Discord role sync and notifications', 0),

    -- Bot credentials
    ('discord.bot_token', '', 'string', 'discord',
     'Discord bot token (from developer portal). Encrypted at rest.', 1),
    ('discord.guild_id', '', 'string', 'discord',
     'Discord server (guild) snowflake ID', 0),

    -- Roles
    ('discord.member_role_id', '', 'string', 'discord',
     'Role ID applied to Active members (and removed from expired ones)', 0),
    ('discord.expired_role_id', '', 'string', 'discord',
     'Role ID applied to Expired/Suspended members ("jail" role)', 0),

    -- Channel IDs (used by D3 notifications — added now so the
    -- settings page is one form rather than two)
    ('discord.events_channel_id', '', 'string', 'discord',
     'Channel ID where new events get posted', 0),
    ('discord.announcements_channel_id', '', 'string', 'discord',
     'Channel ID where new announcements get posted', 0),
    ('discord.admin_alerts_channel_id', '', 'string', 'discord',
     'Channel ID for admin-only alerts (failed payments, integration errors)', 0),

    -- Member onboarding
    ('discord.invite_url', '', 'string', 'discord',
     'Permanent invite URL emailed to newly-activated members', 0),

    -- Connection-test status display
    ('discord.last_test_at', '', 'string', 'discord',
     'When the last test connection was attempted (ISO 8601, empty if never)', 0),
    ('discord.last_test_ok', 'false', 'boolean', 'discord',
     'Whether the last test connection succeeded', 0),
    ('discord.last_test_error', '', 'string', 'discord',
     'Error from the last test (empty on success)', 0);

-- discord_id on members so we know which Discord user to apply roles
-- to. Snowflake format (17–20 ASCII digits) — no DB-level constraint
-- because rolling that out cleanly across SQLite versions is awkward;
-- validation lives in the application layer (see is_valid_snowflake).
ALTER TABLE members ADD COLUMN discord_id TEXT;
