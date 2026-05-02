//! `cmem-server admin ...`:不经 HTTP 直接读写 SQLite。
//!
//! 实现要点:
//! - 复用 server 配置(`server.toml`)读出 `database.path` + `auth.argon2_*`。
//! - 复用 [`auth::password::hash_password`] 做 argon2id。
//! - 所有写操作落 audit_log,action 形如 `admin.user_create` / `admin.invite_create`。

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use clap::Subcommand;
use sqlx::SqlitePool;
use tabled::{settings::Style, Table, Tabled};

use crate::auth::password::hash_password;
use crate::config::AppConfig;
use crate::db::{self, audit, invites, stats, users};

// ---------- CLI ----------

#[derive(Debug, Subcommand)]
pub enum AdminOp {
    /// 用户管理。
    User {
        #[command(subcommand)]
        op: UserOp,
    },
    /// 邀请码管理。
    Invite {
        #[command(subcommand)]
        op: InviteOp,
    },
    /// 全局统计。
    Stats,
    /// 列出审计日志(默认最近 50 条)。
    Audit {
        /// 仅列出该 user 的事件(username,任意大小写)。
        #[arg(long)]
        user: Option<String>,
        /// 最大行数。
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
}

#[derive(Debug, Subcommand)]
pub enum UserOp {
    /// 列出所有用户。
    List,
    /// 创建用户。
    Create {
        #[arg(long)]
        username: String,
        /// 明文密码;若不提供则交互式提示。
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        email: Option<String>,
        /// 直接标记为管理员。
        #[arg(long, default_value_t = false)]
        admin: bool,
    },
    /// 删除用户(级联删除 machines / projects / observations 等)。
    Delete {
        #[arg(long)]
        username: String,
    },
    /// 提升为管理员。
    Promote {
        #[arg(long)]
        username: String,
    },
    /// 取消管理员。
    Demote {
        #[arg(long)]
        username: String,
    },
    /// 禁用用户(is_active = 0)。
    Disable {
        #[arg(long)]
        username: String,
    },
    /// 启用用户(is_active = 1)。
    Enable {
        #[arg(long)]
        username: String,
    },
    /// 重置密码(交互式提示新密码)。
    ResetPassword {
        #[arg(long)]
        username: String,
        /// 跳过交互直接传明文(测试 / 脚本场景)。
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum InviteOp {
    /// 生成邀请码,打印 code。
    Create {
        /// 最大可用次数,默认 1。
        #[arg(long, default_value_t = 1)]
        max_uses: i64,
        /// 过期天数,默认无限期。
        #[arg(long)]
        expires_days: Option<i64>,
    },
    /// 列出所有邀请码及状态。
    List,
    /// 撤销(直接删除)某邀请码。
    Revoke {
        #[arg(long)]
        code: String,
    },
}

// ---------- 入口 ----------

/// 顶层 dispatcher:打开 pool,跑 migration,然后分发。
pub async fn run(config_path: Option<&Path>, op: AdminOp) -> Result<()> {
    let cfg = AppConfig::load_or_default(config_path).context("load config")?;
    let pool = open_pool(&cfg).await?;
    dispatch(&pool, &cfg, op).await
}

/// 打开 sqlite 池 + migrate(admin 子命令复用)。
pub async fn open_pool(cfg: &AppConfig) -> Result<SqlitePool> {
    let pool = db::connect(&cfg.database.path)
        .await
        .with_context(|| format!("open db at {}", cfg.database.path.display()))?;
    db::migrate(&pool).await.context("apply migrations")?;
    Ok(pool)
}

/// 把 op 分发到具体函数。供测试直接复用(传 in-memory pool)。
pub async fn dispatch(pool: &SqlitePool, cfg: &AppConfig, op: AdminOp) -> Result<()> {
    match op {
        AdminOp::User { op } => user_dispatch(pool, cfg, op).await,
        AdminOp::Invite { op } => invite_dispatch(pool, op).await,
        AdminOp::Stats => stats_cmd(pool).await,
        AdminOp::Audit { user, limit } => audit_cmd(pool, user.as_deref(), limit).await,
    }
}

async fn user_dispatch(pool: &SqlitePool, cfg: &AppConfig, op: UserOp) -> Result<()> {
    match op {
        UserOp::List => user_list(pool).await,
        UserOp::Create {
            username,
            password,
            email,
            admin,
        } => {
            let pw = resolve_password(password, "new password: ")?;
            user_create(pool, cfg, &username, &pw, email.as_deref(), admin).await
        }
        UserOp::Delete { username } => user_delete(pool, &username).await,
        UserOp::Promote { username } => user_set_admin(pool, &username, true).await,
        UserOp::Demote { username } => user_set_admin(pool, &username, false).await,
        UserOp::Disable { username } => user_set_active(pool, &username, false).await,
        UserOp::Enable { username } => user_set_active(pool, &username, true).await,
        UserOp::ResetPassword { username, password } => {
            let pw = resolve_password(password, "new password: ")?;
            user_reset_password(pool, cfg, &username, &pw).await
        }
    }
}

async fn invite_dispatch(pool: &SqlitePool, op: InviteOp) -> Result<()> {
    match op {
        InviteOp::Create {
            max_uses,
            expires_days,
        } => invite_create(pool, max_uses, expires_days).await,
        InviteOp::List => invite_list(pool).await,
        InviteOp::Revoke { code } => invite_revoke(pool, &code).await,
    }
}

// ---------- 用户操作 ----------

#[derive(Tabled)]
struct UserRowView {
    id: String,
    username: String,
    admin: &'static str,
    active: &'static str,
    email: String,
    created: String,
    last_login: String,
}

pub async fn user_list(pool: &SqlitePool) -> Result<()> {
    let rows = users::list_all(pool).await.context("list users")?;
    let view: Vec<UserRowView> = rows
        .iter()
        .map(|r| UserRowView {
            id: r.id.clone(),
            username: r.username.clone(),
            admin: if r.is_admin != 0 { "yes" } else { "no" },
            active: if r.is_active != 0 { "yes" } else { "no" },
            email: r.email.clone().unwrap_or_default(),
            created: format_ts(r.created_at),
            last_login: r
                .last_login_at
                .map(format_ts)
                .unwrap_or_else(|| "-".into()),
        })
        .collect();
    print_table(view);
    Ok(())
}

pub async fn user_create(
    pool: &SqlitePool,
    cfg: &AppConfig,
    username: &str,
    password: &str,
    email: Option<&str>,
    is_admin: bool,
) -> Result<()> {
    if users::find_by_username(pool, username)
        .await
        .context("lookup username")?
        .is_some()
    {
        anyhow::bail!("username already taken: {username}");
    }
    let id = uuid::Uuid::now_v7().to_string();
    let now = Utc::now().timestamp();
    let hash = hash_password(password, &cfg.auth).context("hash password")?;
    users::create_user(pool, &id, username, &hash, email, is_admin, now)
        .await
        .context("insert user")?;
    audit::record(
        pool,
        Some(&id),
        None,
        "admin.user_create",
        Some("user"),
        Some(&id),
        None,
        None,
        None,
        now,
    )
    .await
    .context("audit user_create")?;
    println!("created user {username} ({id})");
    Ok(())
}

pub async fn user_delete(pool: &SqlitePool, username: &str) -> Result<()> {
    let user = users::find_by_username(pool, username)
        .await
        .context("lookup username")?
        .ok_or_else(|| anyhow::anyhow!("user not found: {username}"))?;
    let removed = users::delete_by_username(pool, username)
        .await
        .context("delete user")?;
    if !removed {
        anyhow::bail!("delete failed (possibly already removed): {username}");
    }
    audit::record(
        pool,
        Some(&user.id),
        None,
        "admin.user_delete",
        Some("user"),
        Some(&user.id),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .context("audit user_delete")?;
    println!("deleted user {username}");
    Ok(())
}

pub async fn user_set_admin(pool: &SqlitePool, username: &str, is_admin: bool) -> Result<()> {
    let user = users::find_by_username(pool, username)
        .await
        .context("lookup username")?
        .ok_or_else(|| anyhow::anyhow!("user not found: {username}"))?;
    users::set_admin(pool, &user.id, is_admin)
        .await
        .context("update is_admin")?;
    let action = if is_admin {
        "admin.user_promote"
    } else {
        "admin.user_demote"
    };
    audit::record(
        pool,
        Some(&user.id),
        None,
        action,
        Some("user"),
        Some(&user.id),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .context("audit promote/demote")?;
    println!(
        "{} {} -> is_admin={}",
        if is_admin { "promoted" } else { "demoted" },
        username,
        is_admin as i32
    );
    Ok(())
}

pub async fn user_set_active(pool: &SqlitePool, username: &str, is_active: bool) -> Result<()> {
    let user = users::find_by_username(pool, username)
        .await
        .context("lookup username")?
        .ok_or_else(|| anyhow::anyhow!("user not found: {username}"))?;
    users::set_active(pool, &user.id, is_active)
        .await
        .context("update is_active")?;
    let action = if is_active {
        "admin.user_enable"
    } else {
        "admin.user_disable"
    };
    audit::record(
        pool,
        Some(&user.id),
        None,
        action,
        Some("user"),
        Some(&user.id),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .context("audit enable/disable")?;
    println!(
        "{} {} -> is_active={}",
        if is_active { "enabled" } else { "disabled" },
        username,
        is_active as i32
    );
    Ok(())
}

pub async fn user_reset_password(
    pool: &SqlitePool,
    cfg: &AppConfig,
    username: &str,
    new_password: &str,
) -> Result<()> {
    let user = users::find_by_username(pool, username)
        .await
        .context("lookup username")?
        .ok_or_else(|| anyhow::anyhow!("user not found: {username}"))?;
    let hash = hash_password(new_password, &cfg.auth).context("hash password")?;
    users::update_password_hash(pool, &user.id, &hash)
        .await
        .context("update password_hash")?;
    audit::record(
        pool,
        Some(&user.id),
        None,
        "admin.user_reset_password",
        Some("user"),
        Some(&user.id),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .context("audit reset_password")?;
    println!("password reset for {username}");
    Ok(())
}

// ---------- 邀请码操作 ----------

#[derive(Tabled)]
struct InviteRowView {
    code: String,
    max_uses: i64,
    use_count: i64,
    status: &'static str,
    created: String,
    expires: String,
    used_by: String,
}

pub async fn invite_create(
    pool: &SqlitePool,
    max_uses: i64,
    expires_days: Option<i64>,
) -> Result<()> {
    if max_uses < 1 {
        anyhow::bail!("max_uses must be >= 1");
    }
    let code = nanoid::nanoid!(32);
    let now = Utc::now().timestamp();
    let expires_at = expires_days.map(|d| now + d * 86_400);
    invites::create(pool, &code, None, now, expires_at, max_uses)
        .await
        .context("insert invite")?;
    audit::record(
        pool,
        None,
        None,
        "admin.invite_create",
        Some("invite"),
        Some(&code),
        None,
        None,
        None,
        now,
    )
    .await
    .context("audit invite_create")?;
    println!("invite_code: {code}");
    if let Some(exp) = expires_at {
        println!("expires_at:  {}", format_ts(exp));
    }
    println!("max_uses:    {max_uses}");
    Ok(())
}

pub async fn invite_list(pool: &SqlitePool) -> Result<()> {
    let rows = invites::list_all(pool).await.context("list invites")?;
    let now = Utc::now().timestamp();
    let view: Vec<InviteRowView> = rows
        .iter()
        .map(|r| {
            let status = if r.use_count >= r.max_uses {
                "exhausted"
            } else if r.expires_at.map(|e| e <= now).unwrap_or(false) {
                "expired"
            } else {
                "active"
            };
            InviteRowView {
                code: r.code.clone(),
                max_uses: r.max_uses,
                use_count: r.use_count,
                status,
                created: format_ts(r.created_at),
                expires: r
                    .expires_at
                    .map(format_ts)
                    .unwrap_or_else(|| "-".into()),
                used_by: r.used_by.clone().unwrap_or_default(),
            }
        })
        .collect();
    print_table(view);
    Ok(())
}

pub async fn invite_revoke(pool: &SqlitePool, code: &str) -> Result<()> {
    let removed = invites::revoke(pool, code)
        .await
        .context("delete invite")?;
    if !removed {
        anyhow::bail!("invite_code not found: {code}");
    }
    audit::record(
        pool,
        None,
        None,
        "admin.invite_revoke",
        Some("invite"),
        Some(code),
        None,
        None,
        None,
        Utc::now().timestamp(),
    )
    .await
    .context("audit invite_revoke")?;
    println!("revoked invite_code {code}");
    Ok(())
}

// ---------- stats / audit ----------

pub async fn stats_cmd(pool: &SqlitePool) -> Result<()> {
    let s = stats::collect(pool).await.context("collect stats")?;
    println!("users:         {}", s.users);
    println!("machines:      {}", s.machines);
    println!("projects:      {}", s.projects);
    println!("observations:  {}", s.observations);
    println!("active_shares: {}", s.shares);
    println!("invites:       {}", s.invites);
    Ok(())
}

#[derive(Tabled)]
struct AuditRowView {
    id: i64,
    when: String,
    user: String,
    action: String,
    target: String,
}

pub async fn audit_cmd(pool: &SqlitePool, username: Option<&str>, limit: i64) -> Result<()> {
    let user_id = match username {
        Some(name) => {
            let user = users::find_by_username(pool, name)
                .await
                .context("lookup username")?
                .ok_or_else(|| anyhow::anyhow!("user not found: {name}"))?;
            Some(user.id)
        }
        None => None,
    };
    let rows = audit::list_recent(pool, user_id.as_deref(), limit)
        .await
        .context("list audit_log")?;
    let view: Vec<AuditRowView> = rows
        .iter()
        .map(|r| AuditRowView {
            id: r.id,
            when: format_ts(r.created_at),
            user: r.user_id.clone().unwrap_or_default(),
            action: r.action.clone(),
            target: format!(
                "{}:{}",
                r.target_type.clone().unwrap_or_default(),
                r.target_id.clone().unwrap_or_default()
            ),
        })
        .collect();
    print_table(view);
    Ok(())
}

// ---------- 公共工具 ----------

fn format_ts(ts: i64) -> String {
    let dt: DateTime<Utc> = Utc
        .timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap_or_default());
    dt.to_rfc3339()
}

fn print_table<T: Tabled>(rows: Vec<T>) {
    if rows.is_empty() {
        println!("(no rows)");
        return;
    }
    let mut t = Table::new(rows);
    t.with(Style::psql());
    println!("{t}");
}

/// `--password` 优先;否则交互式提示。
fn resolve_password(cli_value: Option<String>, prompt: &str) -> Result<String> {
    if let Some(v) = cli_value {
        return Ok(v);
    }
    let pw = rpassword::prompt_password(prompt).context("read password")?;
    if pw.is_empty() {
        anyhow::bail!("password must not be empty");
    }
    Ok(pw)
}
