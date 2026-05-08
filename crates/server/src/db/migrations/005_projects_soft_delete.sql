-- 005_projects_soft_delete.sql
-- 给 projects 表加软删除支持: 删除项目时不再物理删除,而是 SET deleted_at = unixepoch()。
-- list / get 默认过滤 deleted_at IS NULL;回收站页面查 deleted_at IS NOT NULL。
-- 项目下的 observations 也同步软删(observations 表已有 deleted_at)。

ALTER TABLE projects ADD COLUMN deleted_at INTEGER;
CREATE INDEX idx_projects_deleted_at ON projects(deleted_at);
