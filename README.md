# cmem-server

Rust 实现的 cmem-sync 服务器,用于同步 [claude-mem](https://github.com/thedotmack/claude-mem) 的本地记忆数据库到中心服务器,并在多机和团队成员之间共享。

## 当前状态

- [x] M1 — Workspace 骨架 + SQLite 数据库 schema + `/healthz`
- [x] M2 — 认证(register/login/refresh/logout/change-password,argon2id + JWT)
- [ ] M3 — 机器与同步基础
- [ ] M4 — 项目识别与合并
- [ ] M5 — 项目共享(read-only / fork-allowed / auto-copy)
- [ ] M6 — Fork
- [ ] M7 — CLI 客户端(由 TS 端 claude-mem 直接集成)
- [ ] M8 — 部署 + 文档

## 构建与运行

```bash
# 准备 sqlx 编译期 schema(本地必须有 dev.db)
touch dev.db
DATABASE_URL=sqlite:./dev.db sqlx migrate run --source crates/server/src/db/migrations

# 构建
cargo build --workspace

# 测试
cargo test --workspace

# Lint
cargo clippy --workspace -- -D warnings

# 启动服务器(默认 0.0.0.0:8080,数据库 ./cmem-server.db)
cargo run -p cmem-server
```

健康检查:

```bash
curl http://localhost:8080/healthz
# {"status":"ok","version":"0.1.0"}
```

## 认证流程示例

详细脚本见 `scripts/smoke_auth.sh`。

```bash
# 注册
curl -X POST http://localhost:8080/api/auth/register \
  -H 'content-type: application/json' \
  -d '{"username":"alice","password":"correct horse battery staple"}'

# 登录
curl -X POST http://localhost:8080/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"username":"alice","password":"correct horse battery staple"}'

# 刷新
curl -X POST http://localhost:8080/api/auth/refresh \
  -H 'content-type: application/json' \
  -d '{"refresh_token":"..."}'

# 改密
curl -X POST http://localhost:8080/api/auth/change-password \
  -H "authorization: Bearer $ACCESS" \
  -H 'content-type: application/json' \
  -d '{"old_password":"...","new_password":"..."}'

# 登出
curl -X POST http://localhost:8080/api/auth/logout \
  -H "authorization: Bearer $ACCESS" \
  -H 'content-type: application/json' \
  -d '{"refresh_token":"..."}'
```

## 工程纪律

见 `/Users/bjarne/Code/claude/.scratch/cmem-spec/CLAUDE.md`。要点:

- sqlx 编译期检查 SQL,禁止字符串拼接
- 永远不要 `unwrap()`(测试除外)
- UUID 用 `Uuid::now_v7()`,密码用 argon2id
- `max_connections(1)` SQLite WAL
- machine token 格式 `cmt_<32 nanoid>`,DB 存 SHA-256
