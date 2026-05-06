-- 给 project_paths 加 last_seen_at 列(fork v12.7.2-plus.1 同步,
-- 用于 admin 后台展示"上次活跃路径")
--
-- 默认 created_at(老数据没有 last_seen 信息,用 created_at 兜底)
-- fork sync push 上来的新 path 会带真实 last_seen_at

ALTER TABLE project_paths ADD COLUMN last_seen_at INTEGER NOT NULL DEFAULT 0;

-- 老数据:把 last_seen_at 初始化成 created_at
UPDATE project_paths SET last_seen_at = created_at WHERE last_seen_at = 0;

CREATE INDEX IF NOT EXISTS idx_project_paths_last_seen ON project_paths(last_seen_at DESC);
