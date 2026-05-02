# Contributing

Thanks for considering a contribution. The project is small and
opinionated; this guide tells you how to fit in quickly.

- [Ground rules](#ground-rules)
- [Dev environment](#dev-environment)
- [Building, testing, linting](#building-testing-linting)
- [Code style](#code-style)
- [Commit messages](#commit-messages)
- [Adding a new feature](#adding-a-new-feature)
- [Adding a database column](#adding-a-database-column)
- [Pull request checklist](#pull-request-checklist)
- [Reviewing](#reviewing)
- [Releases](#releases)

---

## Ground rules

- **Keep it small.** This is not a generic SaaS framework. New
  features are weighed against "does it pull a small team out of a
  bind?" If the answer needs a paragraph, it probably belongs in a
  fork.
- **One Rust binary, one SQLite file.** Anything that breaks that rule
  needs strong justification (and a `docs/DECISIONS.md` entry).
- **No `unwrap()` outside tests.** `cargo clippy --workspace --
  -D warnings` enforces it.
- **No string-built SQL.** Every query goes through `sqlx::query!()` /
  `query_as!()` so the schema is type-checked at compile time.
- **Doc what's surprising, not what's obvious.** Comments explain
  *why*, not what `let x = 1;` does.
- **Surgical changes only.** Do not "improve" code that is unrelated
  to your patch. If you genuinely think something nearby is broken,
  open a separate issue or PR.

---

## Dev environment

Requirements:

- `rustup` with the stable toolchain (see `rust-toolchain.toml`)
- `sqlx-cli` for migrations (`cargo install sqlx-cli --no-default-features
  --features rustls,sqlite`)
- `sqlite3` CLI for poking the DB
- `pkg-config` + OpenSSL dev headers (Linux)

Set up the dev DB once so `cargo build` can verify SQL at compile time:

```bash
touch dev.db
DATABASE_URL=sqlite:./dev.db sqlx migrate run \
    --source crates/server/src/db/migrations
```

Editor / IDE:

- **VS Code**: `rust-analyzer` + `Even Better TOML`.
- **Neovim / Helix**: `rust-analyzer` LSP, `gopls`-style completion is
  fine.
- **JetBrains RustRover**: works out of the box.

A `dev-server.toml` with sane localhost defaults is checked in. To run
a dev server pointed at a separate `dev.db`:

```bash
DATABASE_URL=sqlite:./dev.db cargo run -p cmem-server -- -c dev-server.toml
```

---

## Building, testing, linting

```bash
# Format check (must pass in CI)
cargo fmt --all -- --check

# Lint (must pass in CI, zero warnings)
cargo clippy --workspace --all-targets -- -D warnings

# Unit tests
cargo test --workspace

# Run specific test
cargo test -p cmem-server --test sync_flow -- --nocapture

# Build release binary
cargo build --release
```

The `.github/workflows/ci.yml` runs the above on every push and PR.
A red CI is a blocker.

### Smoke scripts (against a running dev server)

```bash
# Terminal 1
DATABASE_URL=sqlite:./dev.db cargo run -p cmem-server -- -c dev-server.toml

# Terminal 2
BASE=http://127.0.0.1:18080 ./scripts/smoke_auth.sh
BASE=http://127.0.0.1:18080 ./scripts/smoke_sync.sh
```

---

## Code style

- Idiomatic Rust 2021 edition.
- `snake_case` modules, `PascalCase` types, descriptive names.
- Module docs (`//!`) at the top of every file explaining the
  responsibility.
- `///` doc comment on every `pub` function and struct so `cargo doc`
  generates a complete reference.
- Comments and module docs in Simplified Chinese (the project AGENTS.md
  rule). Public API docs facing external developers may be in English
  if more idiomatic; mixed is fine.
- Errors:
  - Library code: `Result<T, AppError>` — domain enum, not anyhow.
  - Binaries / CLI: `anyhow::Result` is fine.
  - Wrap with `.context("verb noun")` (lowercase, no punctuation).
- Async:
  - Tokio multi-thread runtime everywhere.
  - Don't hold a `&mut` across `.await` if you can avoid it; pass
    `&AppState` instead.
- HTTP handlers:
  - Extract once with `State<AppState>`, `AuthUser`, `Json<...>`.
  - Return `Result<impl IntoResponse, AppError>`.
  - Audit log every write before the response.

---

## Commit messages

Format used throughout the repo:

```
<area>: <imperative summary, lowercase>

<optional body, hard-wrap at 100 cols>
<explain WHY, not what>

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
```

Common areas: `auth`, `db`, `sync`, `shares`, `projects`, `machines`,
`admin`, `deploy`, `scripts`, `docs`, `ci`, `chore`, `deps`, `tests`.
Examples (from real history):

```
auth: add argon2id password / JWT codec / machine token / Bearer middleware
db: add registration_ip / last_login_ip migration + admin queries
admin: integration tests for admin CLI + invite enforcement
deploy: add systemd unit, server.toml.example and curl smoke script
```

Never amend a commit you've already pushed; instead create a new one.
The repo does not rebase shared branches.

---

## Adding a new feature

1. Open an issue first if the change is non-trivial. Spell out the
   user-visible behaviour change before discussing code.
2. Add a row to `docs/Implementation_Plan.md` so the milestone tracker
   knows about it.
3. Implement in this order:
   - DB migration (one new file under `db/migrations/`).
   - Repository function (`db/<area>.rs`) with sqlx-checked SQL.
   - Domain handler / business logic (`<area>/handlers.rs`).
   - Wire into the router (`server.rs`).
   - Audit log call.
   - Integration test (`crates/server/tests/<area>_flow.rs`).
4. Update `docs/API.md` if the change is HTTP-visible.
5. Update `docs/ARCHITECTURE.md` if it crosses module boundaries.
6. Update `docs/PROJECT_SHARING.md` if it touches sharing semantics.
7. Run the full pre-commit gauntlet (`fmt --check`, `clippy
   -D warnings`, `test --workspace`).

---

## Adding a database column

```bash
N=$(printf '%04d\n' $(($(ls crates/server/src/db/migrations | wc -l) + 1)))
$EDITOR crates/server/src/db/migrations/${N}_add_my_column.sql
DATABASE_URL=sqlite:./dev.db sqlx migrate run --source crates/server/src/db/migrations
cargo build      # sqlx will revalidate every query against the new schema
```

Conventions:

- Migration filename: `NNNN_<verb>_<noun>.sql`.
- Forward-only. **Never** edit a migration that's been merged.
- Default values when adding non-NULL columns to existing tables.
- Add an index in the same migration if the column will be used in
  `WHERE` / `JOIN`.

---

## Pull request checklist

Before opening a PR, verify:

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] New endpoints documented in `docs/API.md`
- [ ] New CLI subcommands documented in `docs/USAGE.md` /
      `docs/ADMIN.md`
- [ ] Audit log written for every state mutation
- [ ] No `unwrap()` outside `#[cfg(test)]`
- [ ] No new dependency without a one-line justification in the PR body
- [ ] Commit messages follow the `area: summary` style above

---

## Reviewing

- Reviewers focus on:
  - Architectural fit (does it belong here?)
  - Sharing-model invariants if the diff touches `shares/`
  - SQL correctness (especially `JOIN`s and soft-delete filtering)
  - Audit log completeness
  - Test coverage for new error paths
- Style nits: leave a comment, but don't block on bikeshed-level
  things.
- For invasive changes, request a `code-reviewer` subagent run; for
  share-mode logic also request a security review (see `code-audit`
  skill).

---

## Releases

```bash
# 1. bump version in Cargo.toml + crates/*/Cargo.toml
# 2. update CHANGELOG.md (move Unreleased -> v0.x.0, add new Unreleased)
# 3. tag
git tag -s v0.x.0 -m "v0.x.0"
git push origin v0.x.0

# 4. CI builds release binaries for x86_64-linux, aarch64-linux,
#    x86_64-darwin, aarch64-darwin and uploads them to the GitHub Release
#    (workflow lives in .github/workflows/release.yml — TODO).
```

There is no release calendar; the project ships when M5 / M6 land
fully and the CI matrix is green.
