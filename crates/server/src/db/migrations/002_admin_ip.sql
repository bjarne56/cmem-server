-- 002: admin web 后台所需的 IP 追踪字段。
--
-- SQLite ALTER TABLE 不支持 IF NOT EXISTS,但 sqlx::migrate! 自带的版本表(_sqlx_migrations)
-- 会保证每个 migration 文件只跑一次,因此这里直接 ALTER。

ALTER TABLE users ADD COLUMN registration_ip TEXT;
ALTER TABLE users ADD COLUMN last_login_ip   TEXT;
