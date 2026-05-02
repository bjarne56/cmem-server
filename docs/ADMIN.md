# cmem-server admin web

服务端内置一套基于 askama + HTMX + Tailwind(CDN)的轻量管理后台,挂在
`/admin` 路径下,鉴权与业务 API 共用同一套 JWT。本文档说明如何启用、各页面
功能、导出格式以及部署到 VPS 上的安全建议。

## 1. 提升第一个 admin

admin web 的鉴权要求当前用户在 `users` 表里 `is_admin = 1 AND is_active = 1`。
但首次部署时数据库里还没有任何 admin,因此必须用以下方法之一手动「凿门」:

```bash
# 方法 A: 直接改 sqlite
sqlite3 cmem-server.db "UPDATE users SET is_admin = 1 WHERE username = 'your_username';"

# 方法 B: 用内置 CLI(已实现 admin user promote 子命令)
cmem-server admin user promote --username your_username
```

之后浏览器打开 `http://127.0.0.1:18080/admin/login`,用该用户登录即可。
登录成功会下发一个 HttpOnly cookie `cmem_admin_session`(其值是普通 access JWT,
有效期沿用 `auth.access_token_ttl_secs`)。

> 注意:`/admin/login` 页面只接受已经被标记为 admin 的用户。普通用户即便密码
> 正确也会被拒绝,以避免泄露「该用户是不是 admin」的信息。

## 2. 路由总览

```
/admin/login              GET  POST   公开,设置 / 校验登录表单(底部含"注册"链接 → /register)
/admin/logout             POST        清 cookie 跳回登录页
/admin                    GET         dashboard(统计 + 24h 趋势)
/admin/users              GET         用户列表 + 模糊搜 + 创建表单
/admin/users/:id          GET         单用户详情(含登录历史)
/admin/invites            GET         邀请码列表 + 创建表单
/admin/projects           GET         全局项目列表 + 按 user / 名称过滤
/admin/observations       GET         全局 observation + FTS 搜索
/admin/shares             GET         全局 share 列表 + 强制 revoke
/admin/audit              GET         审计日志 + 按 user / action 前缀过滤
/admin/export             GET         导出页(下载链接集合)
/admin/settings           GET  POST   服务器设置(注册策略热配置)

/register                 GET  POST   公开注册页(给拿到邀请码的新用户)

/api/admin/stats                 GET
/api/admin/users                 GET POST
/api/admin/users/:id             PATCH DELETE
/api/admin/users/:id/reset-password POST
/api/admin/invites               GET POST
/api/admin/invites/:code         DELETE
/api/admin/projects              GET
/api/admin/observations          GET
/api/admin/observations/:id      DELETE
/api/admin/shares                GET
/api/admin/shares/:id            DELETE
/api/admin/audit                 GET

/api/admin/export/users.csv               GET
/api/admin/export/audit.csv               GET   ?from=&to=
/api/admin/export/observations.csv        GET   ?user=&project=&from=&to=
/api/admin/export/full.db.gz              GET
/api/admin/export/user/:id.zip            GET
```

所有 `/api/admin/*` 与受保护 `/admin/*` 都走 `require_admin` 中间件:

1. 读 `Cookie: cmem_admin_session=<JWT>` 优先,fallback 到
   `Authorization: Bearer <JWT>`。
2. 解码 JWT,要求 `kind = access`。
3. 查 `users` 表,确认 `is_admin = 1 AND is_active = 1`。
4. 失败时根据 `Accept: text/html` 决定:浏览器 GET 跳 `/admin/login`,API 调用
   返回 JSON 401/403。

## 3. 页面速览(ASCII 草图)

```
┌────────────────────────┐
│ cmem-server            │  Dashboard
│ admin console          │  ┌──────┬──────┬──────┬──────┐
│ ──────                 │  │users │mach. │proj. │obs.  │
│ • Dashboard            │  │  5   │ 10   │ 23   │ 410  │
│ • Users                │  └──────┴──────┴──────┴──────┘
│ • Invites              │  ┌─────── 24h activity ───────┐
│ • Projects             │  │ ████ users ███ obs █ audit │
│ • Observations         │  └────────────────────────────┘
│ • Shares               │
│ • Audit Log            │
│ • Export               │
│                        │
│ signed in as root      │
│ [logout]               │
└────────────────────────┘
```

`Users` 表有以下列(从左到右):

```
username | email | admin | active | machines | projects | obs | created
         | last login | last login ip | reg ip | [delete]
```

## 4. 导出格式

| 文件 | 内容 |
| --- | --- |
| `users.csv` | id, username, email, is_admin, is_active, created_at, last_login_at, registration_ip, last_login_ip |
| `audit.csv` | id, user_id, machine_id, action, target_type, target_id, created_at |
| `observations.csv` | id, user_id, username, machine_id, project_id, project_name, timestamp, project_path, obs_type, server_seq, server_received_at, deleted_at, content |
| `cmem-server.db.gz` | `VACUUM INTO` 出来的完整 SQLite 数据库,gzip 压缩。可直接 `gunzip` + `sqlite3` 打开。 |
| `user-<username>-<id>.zip` | 单用户完整数据。每张表一个 JSON 文件:`user.json` / `machines.json` / `projects.json` / `observations.json` / `shares.json` / `audit.json`。 |

每次导出都会写一条 `audit_log.action = admin.export`,`metadata` 里区分 `kind`。

## 5. 安全清单(部署到 VPS 时)

1. **永远只在 HTTPS 后挂 admin web**。本地 `127.0.0.1:18080` 没有 TLS,但生产
   必须放在 Caddy / nginx 后,确保 cookie 不会以明文出现在网络上。
2. **配 Caddy 时给 cookie 补 `Secure` 标志**。当前实现没有自动加,因为本地
   `http://127.0.0.1` 不允许 `Secure`。生产部署可以让 Caddy 在 reverse proxy
   层覆盖 `Set-Cookie`,或者把 `cmem-server` 监听 unix socket 由 Caddy 反代。
3. **别把 `:18080` 直接暴露给公网**,默认监听是 `0.0.0.0`。最起码:
   - bind 改成 `127.0.0.1:18080`,前置 reverse proxy,或
   - 防火墙只允许 admin 自己的出口 IP。
4. **不要把 `cmem-server.db` / `*.db.gz` 之类的备份文件放到 web 可下载位置**。
   `users.csv` / `*.zip` 含 IP 与登录时间等隐私字段。
5. **审计日志保留**:`audit_log` 用 SQLite 表,体积不大,建议长期保留。
   可以定期 `cmem-server admin audit > snapshot.tsv` 备份。
6. **第一个 admin 之后,通过 web 后台管理其他 admin**:不要继续让多个人手工
   `UPDATE users` 改 is_admin。

## 6. 服务器设置(注册策略)

`/admin/settings` 提供唯一一个热配置项:**用户注册策略**。三档单选:

| 档位 | 行为 |
|---|---|
| **open** 开放注册 | 任何人可注册;邀请码可填可不填(填了管理员可追溯来源) |
| **invite_only** 仅邀请码 | 必须有效邀请码,`/register` 强制必填,API 拒无邀请码 |
| **closed** 禁止注册 | `/register` 显示 🚫 停用页;API 直接 400 reject |

切档 → 即时生效,不需要重启。改动写入 `audit` 表,事件名
`admin.settings.registration_mode`,`updated_by` 记录哪个 admin 改的。

底层数据存在 `server_settings` k/v 表(`migration 003_server_settings.sql`)。
首次启动时 lazy init:按 config.toml 的 `[auth].require_invite` 推算
默认值(true → invite_only,false → open),向后兼容。

## 7. 公开注册页 `/register`

不在 `/admin` 下,但走相同的 CSRF + login rate limit 中间件(防 spam)。

新用户使用流:

1. admin 在 `/admin/invites` 创建邀请码,**私下**发给用户
2. 用户浏览器打开 `https://cmem.example.com/register`
3. 填:用户名 / 密码 / 确认密码 / 邮箱(可选)/ **邀请码**
4. 提交成功 → 显示 ✓ "账号已创建,请打开 claude-mem 客户端 → Sync → 用此账号登录"
5. 失败 → 字段回显 + 错误提示(密码不回显)

`/admin/login` 页底部有"新用户?使用邀请码注册"链接 → `/register`。

## 8. 已知限制 / 后续工作

- IP 提取依赖 axum 的 `ConnectInfo`,在反代后面拿到的是 reverse proxy 自身的
  IP。生产建议在 Caddy 层加 `X-Forwarded-For`,然后在 server 里读取(待实现)。
- 模板没有 i18n,全部英文 + 一些中文注释。如需中文 UI 直接改
  `crates/server/templates/*.html`。
- HTMX 的 `+ new user` 表单依赖 `json-enc` 扩展(已经在 `base.html` 里 CDN
  引入)。如果浏览器无法访问 unpkg.com,该按钮不工作 — 可以改成原生
  `<form action="/api/admin/users" method="post">` + 服务端再加一组 form-aware
  handler。运维用 `curl POST application/json` 同样可用。
