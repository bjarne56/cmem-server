-- 003: server-wide 可热更新配置 (key/value),由 admin web 管理。
--
-- 跟 config.toml 的区别:config 是启动时静态加载,改了要重启;
-- server_settings 是 db 行,admin 在 web 上改即时生效。
-- 当前用途:registration_mode (open / invite_only / closed)。
-- 后续可加:max_observations_per_user / default_share_mode 等。

CREATE TABLE IF NOT EXISTS server_settings (
    key         TEXT PRIMARY KEY NOT NULL,
    value       TEXT NOT NULL,
    updated_at  INTEGER NOT NULL,
    updated_by  TEXT  -- user_id,NULL = 系统初始化
);
