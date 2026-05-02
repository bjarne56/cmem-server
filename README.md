# cmem-server

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](rust-toolchain.toml)
[![axum](https://img.shields.io/badge/axum-0.7-brightgreen.svg)](https://github.com/tokio-rs/axum)
[![SQLite](https://img.shields.io/badge/sqlite-3-003B57.svg)](https://www.sqlite.org/)
[![Status](https://img.shields.io/badge/status-alpha-yellow.svg)](docs/Implementation_Plan.md)

> **Self-hosted multi-machine sync + project sharing server for
> [claude-mem](https://github.com/thedotmack/claude-mem).**
> One Rust binary. One SQLite file. argon2id + JWT. systemd / launchd /
> Docker. Built-in admin web console. ~10 MB binary, ~5 MB RSS at idle.

If you use [claude-mem](https://github.com/thedotmack/claude-mem) on more
than one machine, or want to share project memory with teammates,
**cmem-server** is the missing piece: a single-binary server that
synchronises observations across machines and lets you share entire
projects (read-only / fork-allowed / auto-copy) without giving anyone
your local filesystem.

[中文 README](docs/i18n/README.zh.md) · [Install](docs/INSTALL.md) ·
[Deploy](docs/DEPLOYMENT.md) · [Admin web](docs/ADMIN.md) ·
[API](docs/API.md) · [Architecture](docs/ARCHITECTURE.md) ·
[Project sharing](docs/PROJECT_SHARING.md) ·
[Security](docs/SECURITY.md) · [Troubleshooting](docs/TROUBLESHOOTING.md) ·
[Contributing](docs/CONTRIBUTING.md)

---

## Why?

`claude-mem` keeps a brilliant per-machine SQLite database of every
observation Claude Code learns about your projects — but it stops at one
machine. The moment you `ssh` into your VPS, switch from MacBook to
Linux desktop, or want to hand a project context to a teammate, that
local DB is suddenly the wrong DB.

`cmem-server` is a tiny self-hosted daemon that:

- Accepts JSONL pushes of observations from any number of `claude-mem`
  clients (machine token authenticated)
- Re-emits them on `pull` — own observations across machines, plus
  observations shared by other users
- Owns a **project identity layer**: same logical project on Mac
  (`~/work/nginx-rce`) and Linux (`~/projects/nginx-rce`) is one
  project, not two
- Implements three sharing modes (read-only / fork-allowed / auto-copy)
  with mode-downgrade notifications
- Ships a built-in admin web console at `/admin` (askama + HTMX, no SPA
  to babysit)
- Stores everything in **one SQLite file** — back up by `cp`

### Non-goals

- Replacing claude-mem itself
- Multi-tenant SaaS at million-user scale
- Real-time collaborative editing
- Anything that needs Postgres / Redis / Elasticsearch

This is a personal / small-team tool. **Simple beats correct-but-bloated.**

---

## 5-minute quick start

```bash
# On the server (any of: macOS / Ubuntu / Debian / Rocky / Fedora / Arch / Alpine)
git clone https://github.com/bjarne/cmem-server
cd cmem-server
sudo ./scripts/install-server.sh --bind 127.0.0.1:8080
```

That's it. The installer will:

1. Detect your OS
2. Install build deps (`build-essential` / `gcc` / `base-devel` ...)
3. Install Rust via `rustup` (skipped if `cargo` is present)
4. `cargo build --release`
5. Create `cmem` system user + `/var/lib/cmem-server` data dir
6. Generate `/etc/cmem-server.toml` with a fresh 256-bit JWT secret
7. Install + enable a hardened `systemd` unit (or `launchd` plist on macOS)
8. Bootstrap a default admin: **`admin` / `admin@123`** (change immediately!)
9. Wait for `/healthz` to return 200

Open `http://127.0.0.1:8080/admin/login` and log in.

Verify everything is healthy:

```bash
sudo ./scripts/install-server.sh --check
# 8-point check: OS / Rust / binary / config / data dir / service /
# /healthz / admin user. Non-zero exit on any failure (cron-friendly).
```

End users register at `http://127.0.0.1:8080/register` (admin controls
whether registration is **open / invite-only / closed** at
`/admin/settings` — takes effect instantly, no restart).

### Behind a domain (auto HTTPS via Caddy)

```bash
sudo ./scripts/install-server.sh --domain cmem.example.com
```

The installer also drops a Caddy snippet that reverse-proxies your
domain to `127.0.0.1:8080` with auto-issued Let's Encrypt certs.

### Connect a client

On your laptop, point [claude-mem](https://github.com/thedotmack/claude-mem)
at the server (the `--server` flag is consumed by the
`install-client.sh` companion script):

```bash
curl -sSL https://raw.githubusercontent.com/<your>/claude-mem/main/install-client.sh \
    | bash -s -- --server https://cmem.example.com
claude-mem sync login --server https://cmem.example.com
claude-mem sync push
```

---

## Architecture at a glance

```
                   +-------------------------+
                   |   Browser (admin web)   |
                   +-----------+-------------+
                               |
                          HTTPS|  /admin/* + /api/admin/*
                               |
+----------------+    HTTPS    v
|  claude-mem    +-------> +---+--------------------------+
|  (Mac / Linux  |  REST   |   Caddy (reverse proxy +     |
|   / Windows)   <---------+   Let's Encrypt + HTTP/3)    |
+----------------+         +---+--------------------------+
                               |
                          loopback :8080
                               v
                       +-------+-----------+
                       |   cmem-server     |   one Rust binary
                       |   axum + sqlx     |
                       +-------+-----------+
                               |
                               v
                +--------------+-------------+
                |   /var/lib/cmem-server/    |
                |     cmem-server.db (WAL)   |
                |     cmem-server.db-wal     |
                |     cmem-server.db-shm     |
                +----------------------------+
```

Detailed component diagrams: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## Admin web console

Built into the same binary, mounted at `/admin`. No node, no webpack,
no docker-compose. Tailwind via CDN, askama templates, HTMX for
interactivity.

```
+--------------------------+
| cmem-server              |  Dashboard
| admin console            |  +-------+-------+-------+-------+
| ----------               |  | users | mach. | proj. | obs.  |
| > Dashboard              |  |   5   |  10   |  23   |  410  |
|   Users                  |  +-------+-------+-------+-------+
|   Invites                |  +------ 24h activity -------+
|   Projects               |  | ###  users  ##  obs  # log|
|   Observations           |  +---------------------------+
|   Shares                 |
|   Audit Log              |
|   Export                 |
|                          |
| signed in as root        |
| [logout]                 |
+--------------------------+
```

Pages: dashboard, users, invites, projects, observations, shares, audit
log, export (CSV / JSON / full DB dump). Full reference:
[docs/ADMIN.md](docs/ADMIN.md).

---

## REST API surface

| Group | Method + Path | Auth | Notes |
|-------|---------------|------|-------|
| public | `GET /healthz` | — | liveness |
| auth | `POST /api/auth/register` | — | hot-configurable: open/invite-only/closed (see `/admin/settings`) |
| auth | `POST /api/auth/login` | — | returns access + refresh |
| auth | `POST /api/auth/refresh` | refresh JWT | rotate refresh token |
| auth | `POST /api/auth/logout` | access | revokes refresh |
| auth | `POST /api/auth/change-password` | access | re-checks old |
| machines | `POST /api/machines` | access | issues `cmt_<32>` token |
| machines | `GET  /api/machines` | access | list user's machines |
| machines | `DELETE /api/machines/:id` | access | revoke token |
| projects | `GET  /api/projects` | access | list owned + forked |
| projects | `POST /api/projects` | access | explicit create |
| projects | `GET  /api/projects/:id` | access | full detail |
| projects | `PATCH /api/projects/:id` | access (owner) | rename / hide |
| projects | `DELETE /api/projects/:id` | access (owner) | soft delete |
| sync | `POST /api/sync/push` | access or machine | JSONL ingest |
| sync | `POST /api/sync/pull` | access or machine | cursor pagination |
| shares | `POST /api/shares` | access (owner) | user / public / link |
| shares | `GET  /api/shares` | access | shares I created |
| shares | `PATCH /api/shares/:id` | access (owner) | mode change |
| shares | `DELETE /api/shares/:id` | access (owner) | revoke |
| shares | `GET  /api/shared` | access | shares I received |
| shares | `POST /api/shared/notifications/ack` | access | ack downgrades |
| public | `GET /register` + `POST /register` | — | public web sign-up (CSRF + login rate limit) |
| admin | `/api/admin/*` | admin JWT | full surface (16 routes) |
| admin web | `/admin/settings` | admin cookie | hot-toggle registration_mode (open / invite_only / closed) |

Full request / response shapes: [docs/API.md](docs/API.md).

---

## Project sharing in 90 seconds

| Mode | Recipient sees | Recipient can write | Goes through their `pull` |
|------|----------------|---------------------|---------------------------|
| **read-only** | shared view | no | no copy generated |
| **fork-allowed** | shared view | only after `fork` | no copy until fork |
| **auto-copy** | own copy | yes (it's theirs) | yes, every pull |

Mode downgrades (e.g. `fork-allowed` -> `read-only`) raise a notification
on the recipient's next pull and are explicitly acked.

8 invariants, full state matrix, fork semantics:
[docs/PROJECT_SHARING.md](docs/PROJECT_SHARING.md).

---

## Build from source

```bash
# Requirements: rustup stable, sqlite3, OpenSSL dev headers.
git clone https://github.com/bjarne/cmem-server
cd cmem-server

# Cargo workspace, two crates: cmem-shared + cmem-server.
cargo build --workspace --release

# Run tests (uses in-memory SQLite for unit tests).
cargo test --workspace

# Lint (zero warnings policy).
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check

# Run dev server (bind 0.0.0.0:8080, db ./cmem-server.db).
cargo run -p cmem-server -- -c dev-server.toml
```

Smoke scripts under `scripts/`:

```bash
scripts/smoke_auth.sh    # register / login / refresh / change-password / logout
scripts/smoke_sync.sh    # push / pull / project resolve flow
```

---

## Documentation map

| Document | Audience | Topic |
|----------|----------|-------|
| [INSTALL.md](docs/INSTALL.md) | sysadmin | install on every supported OS |
| [DEPLOYMENT.md](docs/DEPLOYMENT.md) | sysadmin | VPS / Docker / k8s, backup, monitoring |
| [ADMIN.md](docs/ADMIN.md) | admin | admin web pages + CSV / DB export |
| [USAGE.md](docs/USAGE.md) | end-user | claude-mem CLI + viewer flows |
| [API.md](docs/API.md) | client author | REST request / response payloads |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | contributor | crate / module layout, data model |
| [PROJECT_SHARING.md](docs/PROJECT_SHARING.md) | contributor | invariants and state machine |
| [SECURITY.md](docs/SECURITY.md) | sysadmin | threat model, hardening checklist |
| [TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) | sysadmin | common failures + recovery |
| [CONTRIBUTING.md](docs/CONTRIBUTING.md) | contributor | dev setup, commit style, code review |

Chinese versions: [docs/i18n/](docs/i18n/).

---

## Status

| Milestone | Scope | State |
|-----------|-------|-------|
| M1 | Workspace + DB schema + `/healthz` | done |
| M2 | Auth (argon2id + JWT + refresh) | done |
| M3 | Machines + machine token | done |
| M4 | Projects + identification + merge | done |
| M5 | Sync push / pull (+ shares scaffolding) | partial |
| M6 | Forks (project + observation) | wip |
| M7 | claude-mem client integration | done (TS-side fork) |
| M8 | Deployment + docs + admin web | done |

See [docs/Implementation_Plan.md](docs/Implementation_Plan.md) for the
authoritative task tracker (kept up to date by docs-maintenance hooks).

---

## License

MIT. See [LICENSE](LICENSE).

## Contact

Issues / PRs welcome at <https://github.com/bjarne/cmem-server>.
For security disclosures see [docs/SECURITY.md](docs/SECURITY.md).
