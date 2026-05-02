-- ============ 用户 ============
CREATE TABLE users (
    id                   TEXT PRIMARY KEY,
    username             TEXT UNIQUE NOT NULL COLLATE NOCASE,
    password_hash        TEXT NOT NULL,
    email                TEXT,
    is_admin             INTEGER NOT NULL DEFAULT 0,
    is_active            INTEGER NOT NULL DEFAULT 1,
    created_at           INTEGER NOT NULL,
    last_login_at        INTEGER,
    settings             TEXT
) STRICT;
CREATE INDEX idx_users_username ON users(username);
CREATE INDEX idx_users_email ON users(email) WHERE email IS NOT NULL;

-- ============ 机器 ============
CREATE TABLE machines (
    id                   TEXT PRIMARY KEY,
    user_id              TEXT NOT NULL,
    name                 TEXT NOT NULL,
    description          TEXT,
    machine_token_hash   TEXT NOT NULL UNIQUE,
    last_seen_at         INTEGER,
    created_at           INTEGER NOT NULL,
    revoked              INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    UNIQUE (user_id, name)
) STRICT;
CREATE INDEX idx_machines_user ON machines(user_id);

-- ============ 项目 ============
CREATE TABLE projects (
    id                   TEXT PRIMARY KEY,
    user_id              TEXT NOT NULL,
    name                 TEXT NOT NULL,
    display_name         TEXT,
    description          TEXT,
    is_excluded          INTEGER NOT NULL DEFAULT 0,
    created_at           INTEGER NOT NULL,
    forked_from_project  TEXT,
    forked_at            INTEGER,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (forked_from_project) REFERENCES projects(id) ON DELETE SET NULL,
    UNIQUE (user_id, name)
) STRICT;
CREATE INDEX idx_projects_user ON projects(user_id);
CREATE INDEX idx_projects_forked_from ON projects(forked_from_project) WHERE forked_from_project IS NOT NULL;

-- ============ 项目 path 别名 ============
CREATE TABLE project_paths (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id           TEXT NOT NULL,
    machine_id           TEXT NOT NULL,
    path                 TEXT NOT NULL,
    project_marker_id    TEXT,
    created_at           INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY (machine_id) REFERENCES machines(id) ON DELETE CASCADE,
    UNIQUE (machine_id, path)
) STRICT;
CREATE INDEX idx_project_paths_project ON project_paths(project_id);

-- ============ Observation ============
CREATE TABLE observations (
    id                   TEXT PRIMARY KEY,
    user_id              TEXT NOT NULL,
    machine_id           TEXT NOT NULL,
    project_id           TEXT,

    timestamp            INTEGER NOT NULL,
    project_path         TEXT,
    content              TEXT NOT NULL,
    obs_type             TEXT,
    metadata             TEXT,

    derived_from         TEXT,
    derivation_chain     TEXT,

    server_seq           INTEGER NOT NULL,
    server_received_at   INTEGER NOT NULL,
    deleted_at           INTEGER,

    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (machine_id) REFERENCES machines(id) ON DELETE CASCADE,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE SET NULL,
    FOREIGN KEY (derived_from) REFERENCES observations(id) ON DELETE SET NULL
) STRICT;
CREATE INDEX idx_obs_user_seq ON observations(user_id, server_seq DESC);
CREATE INDEX idx_obs_project_seq ON observations(project_id, server_seq DESC) WHERE project_id IS NOT NULL;
CREATE INDEX idx_obs_user_proj_path ON observations(user_id, project_path);
CREATE INDEX idx_obs_server_seq ON observations(server_seq);
CREATE INDEX idx_obs_derived ON observations(derived_from) WHERE derived_from IS NOT NULL;

-- ============ 项目共享 ============
CREATE TABLE project_shares (
    id                   TEXT PRIMARY KEY,
    project_id           TEXT NOT NULL,
    sharer_user_id       TEXT NOT NULL,
    target_type          TEXT NOT NULL,
    target_user_id       TEXT,
    share_token          TEXT UNIQUE,
    share_mode           TEXT NOT NULL,
    expires_at           INTEGER,
    created_at           INTEGER NOT NULL,
    revoked_at           INTEGER,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY (sharer_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (target_user_id) REFERENCES users(id) ON DELETE SET NULL,
    CHECK (target_type IN ('user', 'public', 'link')),
    CHECK (share_mode IN ('read-only', 'fork-allowed', 'auto-copy')),
    UNIQUE (project_id, target_type, target_user_id)
) STRICT;
CREATE INDEX idx_shares_project ON project_shares(project_id);
CREATE INDEX idx_shares_target_user ON project_shares(target_user_id) WHERE target_user_id IS NOT NULL;
CREATE INDEX idx_shares_token ON project_shares(share_token) WHERE share_token IS NOT NULL;
CREATE INDEX idx_shares_active ON project_shares(project_id) WHERE revoked_at IS NULL;

-- ============ Mode 降级通知 ============
CREATE TABLE share_mode_downgrades (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id           TEXT NOT NULL,
    target_user_id       TEXT NOT NULL,
    old_mode             TEXT NOT NULL,
    new_mode             TEXT NOT NULL,
    notified_at          INTEGER,
    created_at           INTEGER NOT NULL,
    FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY (target_user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX idx_downgrades_target ON share_mode_downgrades(target_user_id) WHERE notified_at IS NULL;

-- ============ 全局 server_seq 计数器 ============
CREATE TABLE server_seq_counter (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    value   INTEGER NOT NULL
) STRICT;
INSERT INTO server_seq_counter (id, value) VALUES (1, 0);

-- ============ FTS5 全文搜索 ============
CREATE VIRTUAL TABLE observations_fts USING fts5(
    id UNINDEXED,
    user_id UNINDEXED,
    project_id UNINDEXED,
    content,
    project_path,
    tokenize = 'porter unicode61'
);

CREATE TRIGGER obs_fts_insert AFTER INSERT ON observations BEGIN
    INSERT INTO observations_fts(id, user_id, project_id, content, project_path)
    VALUES (new.id, new.user_id, new.project_id, new.content, new.project_path);
END;
CREATE TRIGGER obs_fts_delete AFTER DELETE ON observations BEGIN
    DELETE FROM observations_fts WHERE id = old.id;
END;
CREATE TRIGGER obs_fts_update AFTER UPDATE ON observations BEGIN
    DELETE FROM observations_fts WHERE id = old.id;
    INSERT INTO observations_fts(id, user_id, project_id, content, project_path)
    VALUES (new.id, new.user_id, new.project_id, new.content, new.project_path);
END;

-- ============ Refresh Token ============
CREATE TABLE refresh_tokens (
    token_hash           TEXT PRIMARY KEY,
    user_id              TEXT NOT NULL,
    issued_at            INTEGER NOT NULL,
    expires_at           INTEGER NOT NULL,
    revoked              INTEGER NOT NULL DEFAULT 0,
    user_agent           TEXT,
    ip_address           TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) STRICT;
CREATE INDEX idx_refresh_user ON refresh_tokens(user_id);

-- ============ 邀请码 ============
CREATE TABLE invite_codes (
    code                 TEXT PRIMARY KEY,
    created_by           TEXT,
    created_at           INTEGER NOT NULL,
    expires_at           INTEGER,
    max_uses             INTEGER NOT NULL DEFAULT 1,
    use_count            INTEGER NOT NULL DEFAULT 0,
    used_by              TEXT,
    used_at              INTEGER,
    FOREIGN KEY (created_by) REFERENCES users(id),
    FOREIGN KEY (used_by) REFERENCES users(id)
) STRICT;

-- ============ 审计日志 ============
CREATE TABLE audit_log (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id              TEXT,
    machine_id           TEXT,
    action               TEXT NOT NULL,
    target_type          TEXT,
    target_id            TEXT,
    metadata             TEXT,
    ip_address           TEXT,
    user_agent           TEXT,
    created_at           INTEGER NOT NULL
) STRICT;
CREATE INDEX idx_audit_user_time ON audit_log(user_id, created_at DESC);
CREATE INDEX idx_audit_action ON audit_log(action, created_at DESC);
