# cmem-server admin web(中文)

> 这是 [docs/ADMIN.md](../ADMIN.md) 的精简中文版。原文档已经是中文,
> 这里主要补充对国内运维场景常见问题的说明。完整页面与导出格式见原文。

服务端内置一套基于 askama + HTMX + Tailwind(CDN)的轻量管理后台,
挂在 `/admin` 路径下,鉴权与业务 API 共用同一套 JWT。

---

## 1. 第一个 admin

admin web 要求当前用户在 `users` 表里 `is_admin = 1 AND is_active = 1`。
但首次部署时数据库里还没有任何 admin。两种方式手动「凿门」:

```bash
# 方法 A:直接改 sqlite
sudo sqlite3 /var/lib/cmem-server/cmem-server.db \
    "UPDATE users SET is_admin = 1 WHERE username = 'your_username';"

# 方法 B:用内置 CLI
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin user promote --username your_username
```

`install-server.sh` 默认会替你做完这一步,创建 `admin / admin@123`。
**部署完立刻改密**:

```bash
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin user reset-password --username admin
```

之后浏览器打开 `https://cmem.example.com/admin/login`(本地是
`http://127.0.0.1:8080/admin/login`)。

---

## 2. 路由速览

```
/admin/login              GET POST    登录表单
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
/api/admin/...                        对应的 REST API(详见 docs/API.md)
```

所有 `/api/admin/*` 与受保护 `/admin/*` 都走 `require_admin` 中间件:

1. 优先读 cookie `cmem_admin_session=<JWT>`,fallback `Authorization: Bearer`。
2. 解码 JWT,要求 `kind = access`。
3. 查 `users` 表,确认 `is_admin = 1 AND is_active = 1`。
4. 失败时按 `Accept: text/html` 区分:浏览器跳 `/admin/login`,API 返回 401/403。

---

## 3. 页面速览

```
+------------------------+
| cmem-server            |  Dashboard
| admin console          |  +-------+-------+-------+-------+
| ----                   |  | users | mach. | proj. | obs.  |
| > Dashboard            |  |   5   |  10   |  23   |  410  |
|   Users                |  +-------+-------+-------+-------+
|   Invites              |  +------ 24h activity -------+
|   Projects             |  | ###  users  ##  obs  # log|
|   Observations         |  +---------------------------+
|   Shares               |
|   Audit Log            |
|   Export               |
|                        |
| signed in as root      |
| [logout]               |
+------------------------+
```

`Users` 表列(从左到右):

```
username | email | admin | active | machines | projects | obs | created
         | last login | last login ip | reg ip | [delete]
```

---

## 4. 导出格式

| 文件 | 内容 |
| ---- | ---- |
| `users.csv` | id, username, email, is_admin, is_active, created_at, last_login_at, registration_ip, last_login_ip |
| `audit.csv` | id, user_id, machine_id, action, target_type, target_id, created_at |
| `observations.csv` | id, user_id, username, machine_id, project_id, project_name, timestamp, project_path, obs_type, server_seq, server_received_at, deleted_at, content |
| `cmem-server.db.gz` | `VACUUM INTO` 出来的完整 SQLite 数据库,gzip 压缩。可直接 `gunzip` + `sqlite3` 打开 |
| `user-<username>-<id>.zip` | 单用户完整数据。每张表一个 JSON 文件:`user.json` / `machines.json` / `projects.json` / `observations.json` / `shares.json` / `audit.json` |

每次导出都会写一条 `audit_log.action = admin.export`,`metadata` 里区
分 `kind`。

---

## 5. 国内 VPS 部署清单

按重要性排序:

1. **必须 HTTPS**:`/admin/*` 下发 cookie,本地 `http://127.0.0.1` 没
   TLS 没问题,生产暴露公网必须挂 Caddy / nginx。
2. **bind 用 127.0.0.1:8080**,反代到位前别监听 `0.0.0.0`。
3. **`require_invite = true`**:除你之外的人能访问 `/api/auth/register`
   时一定要打开。
4. **数据库每天备份**:见 [DEPLOYMENT.md#backups](../DEPLOYMENT.md#backups)。
5. **关注 `audit_log`**:`auth.login_failed` 频次、`admin.user_create`
   / `admin.user_promote` 异常增长都需要警觉。
6. **`jwt_secret` 漏了立刻轮换**(见 [SECURITY.md](../SECURITY.md))。

---

## 6. 已知限制

- IP 提取依赖 axum 的 `ConnectInfo`,在反代后面拿到的是 reverse proxy
  自身的 IP。生产建议 Caddy 加 `X-Forwarded-For`,然后在 server 里读
  `X-Real-IP` 优先(已实现)。
- 模板没有 i18n,全部英文 + 一些中文注释。如需中文 UI 直接改
  `crates/server/templates/*.html`。
- HTMX 的「new user」表单依赖 `json-enc` 扩展(`base.html` CDN 引入)。
  如果浏览器无法访问 unpkg.com,该按钮不工作 —— 可以改成原生
  `<form action="/api/admin/users" method="post">`。运维用 `curl
  POST application/json` 同样可用。
