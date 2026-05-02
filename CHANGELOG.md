# Changelog

All notable changes to **cmem-server** are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Universal `scripts/install-server.sh` — one-shot installer for macOS,
  Ubuntu, Debian, Rocky 8/9, CentOS, Fedora, Arch and Alpine. Detects
  OS, installs build deps, runs `rustup`, builds release binary,
  generates a hardened `cmem-server.toml` (256-bit JWT secret), creates
  the `cmem` system user, registers a hardened `systemd` unit (or
  `launchd` plist on macOS), waits for `/healthz`, optionally
  bootstraps `admin/admin@123` and configures Caddy with auto HTTPS.
- Companion `scripts/uninstall-server.sh` with interactive confirmation,
  `--keep-data`, `--backup`, and `--yes` modes.
- `deploy/launchd/com.cmem.server.plist` — macOS service template.
- `deploy/caddy/Caddyfile.example` — reverse-proxy template with HSTS
  and X-Forwarded-For propagation.
- Multi-stage `Dockerfile` plus `docker-compose.yml` and `.dockerignore`
  for container-friendly deployments.
- `.github/workflows/ci.yml` — `cargo fmt --check`, `cargo clippy
  -D warnings`, `cargo test --workspace` on push / PR.
- Public documentation set under `docs/`:
  - `INSTALL.md`, `DEPLOYMENT.md`, `USAGE.md`, `API.md`,
    `ARCHITECTURE.md`, `PROJECT_SHARING.md`, `SECURITY.md`,
    `TROUBLESHOOTING.md`, `CONTRIBUTING.md`
  - Chinese translations under `docs/i18n/`
- Top-level `README.md`, `LICENSE` (MIT) and `CHANGELOG.md`.

### Changed

- `deploy/systemd/cmem-server.service` — `ExecStart` adjusted to the new
  `INSTALL_PREFIX` (`/opt/cmem-server`) and `CONFIG_PATH`
  (`/etc/cmem-server.toml`) used by `install-server.sh`. Hardening
  expanded with `Protect{Kernel,Control}*`, `RestrictAddressFamilies`,
  `LimitNOFILE=65536`.

### Engineering history (pre-doc squash)

For the detailed module-level history, see `git log` — every milestone
has its own commit prefix:

```
fac8516  admin: web console (middleware + REST API + askama templates + data export)
71c7fde  admin: integration tests for admin CLI + invite enforcement
1843457  admin: add CLI subcommands for users / invites / stats / audit
a0b80da  deps: add askama / askama_axum / csv / zip / flate2 / cookie / tempfile
55d032a  auth: capture client IP on register/login
94215e8  db: add registration_ip / last_login_ip migration + admin queries
1556807  projects: add identification algorithm and CRUD endpoints
73f7c39  machines: add POST/GET/DELETE /api/machines endpoints
6a3624a  db: add repositories for machines/observations/projects/shares
661d9be  shared: add API DTOs and models for machines/projects/sync/shares
75d8b81  deploy: add systemd unit, server.toml.example and curl smoke script
6b3e5ac  server: wire axum router with /healthz and /api/auth/* endpoints
734f733  auth: add argon2id password / JWT codec / machine token / Bearer middleware
95acaa8  db: add connection pool, repositories and AppError
a20d3e8  db: add full SQLite schema for users/machines/projects/obs/shares
285abb7  shared: define cross-crate API types and ShareMode
0fe7118  workspace: init Cargo workspace skeleton
```

## [0.1.0] — TBD (first tagged release)

The first cut targets:

- Stable auth (M1 + M2)
- Full sync push / pull semantics with project identification (M3 + M4)
- All three sharing modes wired end-to-end (M5)
- Project + single observation forks (M6)
- One-shot install on every supported OS
- Admin web console with CSV / JSON / full DB export
- Documentation set listed above

[Unreleased]: https://github.com/bjarne/cmem-server/compare/v0.1.0...HEAD
[0.1.0]:      https://github.com/bjarne/cmem-server/releases/tag/v0.1.0
