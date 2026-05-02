# Architecture

A guided tour of the cmem-server codebase. Aimed at contributors and
people who need to debug behaviour without reading the whole tree.

- [Crates and modules](#crates-and-modules)
- [Process model](#process-model)
- [Request lifecycle](#request-lifecycle)
- [Data model](#data-model)
- [Sync model](#sync-model)
- [Sharing model](#sharing-model)
- [Why these choices](#why-these-choices)
- [Where to look for X](#where-to-look-for-x)

---

## Crates and modules

Single Cargo workspace, two crates:

```
cmem-server/
├── Cargo.toml                         # workspace root
├── crates/
│   ├── shared/                        # cross-crate types (DTOs, ShareMode, ...)
│   │   └── src/
│   │       ├── api.rs                 # request / response shapes
│   │       └── lib.rs
│   └── server/                        # binary + library
│       └── src/
│           ├── main.rs                # tracing init + dispatch
│           ├── lib.rs                 # public modules
│           ├── server.rs              # build_router(state)
│           ├── state.rs               # AppState (pool + cfg)
│           ├── config.rs              # AppConfig load_or_default
│           ├── error.rs               # AppError + HTTP mapping
│           ├── commands/              # CLI dispatcher
│           │   ├── serve.rs           # default subcommand
│           │   └── admin.rs           # `admin user|invite|stats|audit ...`
│           ├── auth/                  # M2
│           │   ├── handlers.rs        # /api/auth/*
│           │   ├── jwt.rs
│           │   ├── tokens.rs          # access / refresh / machine token
│           │   └── password.rs        # argon2id wrappers
│           ├── middleware/
│           │   └── auth.rs            # Bearer extractor + AuthUser
│           ├── machines/              # M3
│           ├── projects/              # M4
│           │   ├── handlers.rs
│           │   └── identification.rs  # resolve_project()
│           ├── sync/                  # M5 push / pull
│           ├── shares/                # M5/M6
│           ├── admin/                 # admin REST + web
│           │   ├── handlers.rs        # /api/admin/*
│           │   ├── middleware.rs      # require_admin
│           │   └── web/               # askama templates + cookies + export
│           └── db/                    # repositories
│               ├── mod.rs             # connect, migrate, pool config
│               ├── users.rs
│               ├── machines.rs
│               ├── projects.rs
│               ├── observations.rs
│               ├── shares.rs
│               ├── invites.rs
│               ├── audit.rs
│               ├── tokens.rs
│               ├── stats.rs
│               └── migrations/        # *.sql files (sqlx-managed)
└── deploy/
    ├── systemd/cmem-server.service
    ├── launchd/com.cmem.server.plist
    ├── caddy/Caddyfile.example
    └── config/server.toml.example
```

Why the split:

- `shared` lets the (future, separate) Rust client crate consume DTOs
  without pulling axum / sqlx.
- `server` library + binary keeps `main.rs` to ~10 lines and makes
  integration tests easy (`cargo test -p cmem-server` builds the
  library, not the binary).

---

## Process model

```
              +------------------+
              |  cmem-server     |
              |  (Tokio runtime) |
              +--------+---------+
                       |
   +-------------------+-------------------+
   |                   |                   |
+--v---+         +-----v------+      +-----v-----+
| axum |         |   sqlx     |      | tracing   |
| router|        |   pool     |      | subscriber|
+--+---+         +-----+------+      +-----+-----+
   |                   |                   |
   |                   v                   v
   |         +---------+--------+    journald / stdout
   |         |  SQLite (WAL)    |
   |         |  max_conns = 1   |
   |         +------------------+
   v
HTTP :8080
```

A single tokio runtime, one axum router, **one sqlx connection** (WAL
mode forces serial writes anyway — multiple connections deadlock under
load on SQLite). All request handlers borrow that connection from a
`SqlitePool::max_connections(1)` pool.

The CLI (`cmem-server admin ...`) opens the **same** SQLite file with
the same migration runner — no daemon required for offline admin work.

---

## Request lifecycle

```
client request
      |
      v
[axum router]                    -- /api/* and /admin/*
      |
      v
[middleware]
      |
      +--> public routes (healthz, /api/auth/{register,login,refresh})
      |
      +--> require_auth   --> AuthUser (user_id + machine_id?)
      |        |
      |        v
      |     handler --(borrows pool)--> sqlx query!()
      |                                    |
      |                                    v
      |                              SQLite tx
      |
      +--> require_admin --> same as require_auth, plus is_admin = 1
                              |
                              v
                            admin handler  (UI templates via askama,
                                            JSON via serde_json)
```

Errors bubble up as `AppError` (`error.rs`). `IntoResponse` converts
them to a uniform `{ "error": ..., "message": ... }` JSON body with
the right HTTP status. Panics inside handlers are caught by axum's
default catch-unwind layer; we additionally enforce "no `unwrap()` in
non-test code" via a `cargo clippy --workspace -- -D warnings` rule.

---

## Data model

The schema lives under `crates/server/src/db/migrations/`. Tables (with
just the relationship-bearing columns):

```
users                 --< machines             --< project_paths
   |                                              |
   +--<  refresh_tokens                           |
   |                                              |
   +--<  invites (created_by)                     |
   |                                              |
   +--<  projects  ---<  observations  --<  shared_views (read-only)
            |                |
            +--<  project_shares --<  share_mode_downgrades
            +--<  project_paths

audit_log (user_id, machine_id?, action, target_*)  -- ledger of everything
```

Important columns:

- `users.id` — UUID v7
- `machines.id` — UUID v7; `token_sha256` (no plaintext stored)
- `projects(user_id, normalized_name)` UNIQUE — auto-merge invariant
- `observations.server_seq` — monotonic per server, assigned in tx
- `observations.deleted_at` — soft delete; all queries filter
- `project_shares.revoked_at` — soft delete
- `share_mode_downgrades(share_id, old_mode, new_mode, ack_at)` —
  notification queue

Migrations are applied at every server / CLI start via
`sqlx::migrate!()`; rolling forward is automatic, rollbacks are not
supported (write a forward-only fix migration instead).

---

## Sync model

```
client (push)                           server                          client (pull)

  collect new obs                          .                               .
  build JSONL  --POST /api/sync/push---->  parse line                      .
                                           resolve_project()                .
                                           INSERT OR IGNORE obs            .
                                           assign server_seq               .
                                           upsert project_paths            .
                                           bump machine.last_seen_at       .
                                           write audit_log                 .
            <--{accepted, dups, ...}--                                     .
                                                                          .
                                                                  POST /api/sync/pull
                                                                  { since_seq, limit, ... }
                                                                          .
                                                  load own_obs WHERE      .
                                                  user_id = me            .
                                                  AND server_seq > since  .
                                                  AND machine_id NOT IN exclude
                                                                          .
                                                  load shared_obs from    .
                                                  active project_shares   .
                                                                          .
                                                  load pending downgrades .
                                                       <--{own, shared, downgrades, next_seq}--
```

`server_seq` monotonicity is enforced inside the same transaction as
the `INSERT` (read-then-write under WAL with `BEGIN IMMEDIATE`).

`resolve_project` (`projects/identification.rs`) — the single function
every push round-trips through:

1. If client supplied `project_marker_id` → look it up; trust it if it
   belongs to `user_id`.
2. Else normalise `project_name` → look up `(user_id,
   normalized_name)`. Match → reuse.
3. Else → create a new project owned by the user.

This is **the** project-merging contract. Same name on Mac and Linux
collapses into one project; different names stay separate. Override
with `.cmem-project.toml` for tighter coupling.

---

## Sharing model

Three modes (read-only / fork-allowed / auto-copy) × eight invariants
laid out in [PROJECT_SHARING.md](PROJECT_SHARING.md). The server-side
implementation lives in `crates/server/src/shares/`.

Key principle: **the data owner's table is canonical**. A "share" is
just a row in `project_shares` plus query-time joins. Recipients never
mutate the source observations; they either get a transient view
(`shared_view`) or a server-coordinated copy that becomes their own
data (`auto-copy` / `fork`).

A mode downgrade is **not** a delete. The server appends to
`share_mode_downgrades`; the recipient sees the notification on next
pull and acks it. Already-forked or auto-copied data is theirs forever.

---

## Why these choices

### Why SQLite?

- One file backup story.
- Zero ops at our scale (< 1 M observations, < 100 active users).
- WAL mode handles single-writer + many-reader trivially.
- Migrating to Postgres is a 2-day port if you ever outgrow it.

### Why axum?

- Smallest, most idiomatic Tokio HTTP framework.
- `Router::nest`, middleware stacks, extractors all compose cleanly.
- Used in production by 100+ Rust shops, well-supported.

### Why argon2id with RFC 9106 defaults?

- 19 MiB / 2 iter / 1 thread is OWASP recommended.
- ~100 ms per login is enough to make brute force expensive without
  killing UX.

### Why a single SqlitePool connection?

- WAL serialises writes anyway. Multiple connections amplify lock
  contention and produce `database is locked` under load.
- Reads still go through the same connection; SQLite is fast enough.

### Why no Redis / RabbitMQ / Kafka?

- We don't need a broker. Push is synchronous; pull is polled (or
  long-poll in a future iteration). Anything more is over-engineered
  for the load profile this tool is built for.

### Why askama + HTMX, not React/Vue?

- The admin web has < 10 pages. A SPA is overkill.
- askama renders templates at compile time — typos break `cargo build`.
- HTMX handles all the "send a form, replace this fragment" cases we
  need.
- Tailwind via CDN keeps it zero-toolchain.

---

## Where to look for X

| You want to ... | Look at |
|-----------------|---------|
| Add a new HTTP route | `crates/server/src/server.rs` (`build_router`) |
| Wire a new admin page | `crates/server/src/admin/web/` |
| Change password hashing | `crates/server/src/auth/password.rs` |
| Tweak the project-merge algorithm | `crates/server/src/projects/identification.rs` |
| Add a new share mode | `crates/shared/src/api.rs` (`ShareMode`) + `shares/handlers.rs` + state matrix in [PROJECT_SHARING.md](PROJECT_SHARING.md) |
| Add a CLI subcommand | `crates/server/src/commands/admin.rs` |
| Add a new admin export | `crates/server/src/admin/web/export.rs` |
| Change how machines authenticate | `crates/server/src/middleware/auth.rs` |
| Add a database column | new file under `crates/server/src/db/migrations/` |

The codebase aims to keep **one concept per file** — `users.rs` only
deals with users, `shares.rs` only with shares. If you find yourself
crossing module boundaries inside one function, that's the signal to
introduce a new module.
