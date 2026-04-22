-- Add scope and toml_path to agent_profiles.
-- scope: 'global' | 'workspace' — determines where the TOML file lives.
-- toml_path: absolute path to the source TOML file (NULL for profiles created only in DB).
ALTER TABLE agent_profiles ADD COLUMN scope TEXT NOT NULL DEFAULT 'global';
ALTER TABLE agent_profiles ADD COLUMN toml_path TEXT;
