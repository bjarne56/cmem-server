# cmem-server REST API

All endpoints serve JSON unless noted. Authentication uses bearer
tokens (`Authorization: Bearer <jwt>` for users, `Authorization: Bearer
cmt_<32>` for machines). Errors follow a uniform shape:

```json
{ "error": "code", "message": "human readable", "details": {} }
```

| HTTP | Meaning |
|------|---------|
| 200 / 201 | success |
| 400 | bad input (validation, malformed JSON) |
| 401 | missing / invalid token |
| 403 | authenticated but not allowed |
| 404 | resource not found (or visible) |
| 409 | conflict (duplicate username, etc.) |
| 410 | revoked share or deleted resource |
| 413 | payload too large (8 MB cap) |
| 422 | unprocessable (e.g. share mode invalid) |
| 429 | rate-limited |
| 500 | server bug — please file an issue |

Token lifetimes are config-driven (defaults below). All request
bodies are validated; max body size is **8 MiB** for JSON, **32 MiB**
for sync push (override via reverse proxy `client_max_body_size`).

---

## Conventions

- Times: RFC 3339 UTC strings (`2026-05-02T13:14:15Z`).
- IDs: UUID v7 strings; sortable by time.
- Pagination: cursor-based via `since_seq` / `next_since_seq`
  (sync) or `?limit=N&offset=M` (admin).
- Field names: snake_case in JSON.
- All write endpoints log to `audit_log`.

---

## Public endpoints

### `GET /healthz`

Liveness probe.

```bash
curl http://127.0.0.1:8080/healthz
```

```json
{ "status": "ok", "version": "0.1.0" }
```

---

## Auth

### `POST /api/auth/register`

Create a new user.

Request:

```json
{
  "username": "alice",
  "password": "correct horse battery staple",
  "email": "alice@example.com",
  "invite_code": "abc123"
}
```

| Field | Required | Notes |
|-------|----------|-------|
| `username` | yes | 3–64 chars, `[a-zA-Z0-9_-]+`, unique |
| `password` | yes | min 8 chars; argon2id-hashed before storage |
| `email`    | no  | optional, no uniqueness constraint |
| `invite_code` | conditional | required when `[auth].require_invite = true` |

Response 201:

```json
{
  "user": {
    "id": "019...",
    "username": "alice",
    "email": "alice@example.com",
    "created_at": "2026-05-02T..."
  }
}
```

Errors: 409 `username_taken`, 422 `invite_required`, 400 `invite_invalid`.

### `POST /api/auth/login`

```json
{ "username": "alice", "password": "..." }
```

Response 200:

```json
{
  "user": { "id": "...", "username": "alice", "email": "...", "created_at": "..." },
  "access_token":  "eyJhbGciOi...",
  "access_token_expires_at": "2026-05-02T13:29:00Z",
  "refresh_token": "eyJhbGciOi..."
}
```

Failure (any reason — wrong username, wrong password, disabled user)
returns the **same** 401 message to avoid user enumeration.

### `POST /api/auth/refresh`

```json
{ "refresh_token": "..." }
```

Returns a fresh access + refresh pair. The old refresh token is
revoked on success (rotate-on-use).

### `POST /api/auth/logout` *(access)*

```json
{ "refresh_token": "..." }
```

Revokes the supplied refresh token; the access token remains valid
until its TTL expires.

### `POST /api/auth/change-password` *(access)*

```json
{ "old_password": "...", "new_password": "..." }
```

Returns 200 on success. All refresh tokens for the user are revoked
server-side.

---

## Machines

A "machine" represents one device's persistent identity. Each one is
issued a `cmt_<32 nanoid>` token used by `claude-mem sync push/pull` to
authenticate without a user JWT round-trip.

### `POST /api/machines` *(access)*

```json
{ "name": "alice-mac", "description": "MacBook Pro M3" }
```

Response 201:

```json
{
  "machine": {
    "id": "019...",
    "name": "alice-mac",
    "description": "MacBook Pro M3",
    "created_at": "...",
    "last_seen_at": null
  },
  "machine_token": "cmt_aBcDeFgHiJ..."
}
```

The `machine_token` is shown **once** — store it on the device.

### `GET /api/machines` *(access)*

```json
{
  "machines": [
    { "id": "...", "name": "alice-mac", "last_seen_at": "...", "created_at": "..." }
  ]
}
```

### `DELETE /api/machines/:id` *(access)*

Revokes the machine's token. The device must re-register.

---

## Projects

### `GET /api/projects` *(access)*

Returns projects owned by the user plus those forked from shares.

```json
{
  "projects": [
    {
      "id": "019...",
      "name": "nginx-rce",
      "display_name": null,
      "description": "...",
      "is_excluded": false,
      "forked_from": null,
      "observation_count": 127,
      "paths": [
        { "machine_id": "...", "machine_name": "alice-mac",   "path": "/Users/alice/work/nginx-rce" },
        { "machine_id": "...", "machine_name": "alice-linux", "path": "/home/alice/projects/nginx-rce" }
      ],
      "shares": [
        {
          "id": "...",
          "target_type": "user",
          "target_user": { "id": "...", "username": "bob" },
          "share_mode": "fork-allowed",
          "created_at": "..."
        }
      ],
      "created_at": "..."
    }
  ]
}
```

### `POST /api/projects` *(access)*

Explicitly create a project (corresponds to `cmem-sync project init`).

```json
{ "name": "nginx-rce", "description": "exploit chain notes" }
```

Returns the created project. Client writes the `id` into a
`.cmem-project.toml` so the project marker is stable across machines.

### `GET /api/projects/:id` *(access)*

Same shape as the list entry. 404 if not visible to the caller.

### `PATCH /api/projects/:id` *(access, owner)*

```json
{ "name": "...", "display_name": "...", "description": "...", "is_excluded": true }
```

Only the owner can patch. `is_excluded = true` hides the project from
default lists (useful for archival without delete).

### `DELETE /api/projects/:id` *(access, owner)*

Soft-deletes the project and all its observations. Hard-delete only
via admin CLI (`admin project purge`).

---

## Sync

### `POST /api/sync/push` *(access or machine)*

JSONL body — one observation per line:

```jsonl
{"id":"019...","timestamp":1714583400,"project_marker_id":"019...","project_name":"nginx-rce","project_path":"/Users/alice/work/nginx-rce","content":"...","obs_type":"decision","metadata":{},"derived_from":null}
{"id":"019...","timestamp":1714583500,"project_marker_id":null,"project_name":"55ai","project_path":"/Users/alice/work/55ai","content":"...","obs_type":"observation"}
```

Server steps (per observation):

1. Resolve project ID via `(user_id, marker_id, project_name,
   project_path)` (see [PROJECT_SHARING.md](PROJECT_SHARING.md)).
2. `INSERT OR IGNORE` into `observations`, assign `server_seq` inside
   the transaction.
3. Upsert `project_paths(machine_id, project_id, path)`.
4. `UPDATE machines SET last_seen_at = now()`.
5. Append to `audit_log`.

Response 200:

```json
{
  "accepted": 95,
  "duplicates": 5,
  "errors": [],
  "server_seq_max": 12345,
  "projects_resolved": [
    { "submitted_name": "nginx-rce", "project_id": "019..." },
    { "submitted_name": "55ai",      "project_id": "019..." }
  ]
}
```

`projects_resolved` lets the client persist server-assigned `project_id`
into `.cmem-project.toml`.

### `POST /api/sync/pull` *(access or machine)*

```json
{
  "since_seq": 12000,
  "limit": 500,
  "include_shared": true,
  "include_public": false,
  "exclude_machines": ["019..."]
}
```

`exclude_machines` is typically the caller's own `machine_id`, to avoid
echoing back what the client just pushed.

Response 200:

```json
{
  "own_observations":    [/* Observation[] */],
  "shared_observations": [
    {
      "observation": { /* Observation */ },
      "share_mode": "fork-allowed",
      "sharer_user_id":   "...",
      "sharer_username":  "alice",
      "project_id":       "...",
      "project_name":     "nginx-rce"
    }
  ],
  "pending_downgrades": [
    {
      "share_id": "...",
      "project_id": "...",
      "project_name": "nginx-rce",
      "old_mode": "fork-allowed",
      "new_mode": "read-only",
      "downgraded_at": "..."
    }
  ],
  "next_since_seq": 12345,
  "has_more": true
}
```

Clients call `pull` with `since_seq = next_since_seq` until `has_more =
false`.

### `POST /api/shared/notifications/ack` *(access)*

```json
{ "share_ids": ["...", "..."] }
```

Acknowledges seen downgrade notices so they stop appearing in
subsequent `pull` responses.

---

## Shares

### `POST /api/shares` *(access, owner)*

```json
{
  "project_id": "019...",
  "target_type": "user",
  "target_username": "bob",
  "share_mode": "fork-allowed",
  "expires_in_secs": 604800
}
```

| `target_type` | extra fields | recipient |
|---------------|--------------|-----------|
| `user`   | `target_username` | one named user |
| `public` | (none) | any logged-in user |
| `link`   | (none) | anonymous holder of `share_token` |

Response 201:

```json
{
  "share": {
    "id": "...",
    "target_type": "user",
    "target_user": { "id": "...", "username": "bob" },
    "share_mode": "fork-allowed",
    "share_token": null,
    "created_at": "..."
  },
  "share_url": null
}
```

For `target_type = "link"` the response includes
`"share_url": "https://cmem.example.com/p/<32-char-token>"`.

### `GET /api/shares` *(access)*

Shares the caller has **created**.

### `PATCH /api/shares/:id` *(access, owner)*

```json
{ "share_mode": "read-only", "expires_at": "2026-06-01T..." }
```

Only `share_mode` and `expires_at` are mutable. A downgrade
(e.g. `fork-allowed → read-only`) appends a row to
`share_mode_downgrades`; the recipient sees it on next `pull`.

### `DELETE /api/shares/:id` *(access, owner)*

Revokes the share. The recipient's next `pull` clears their
`shared_view`; **already forked / auto-copied data stays**.

### `GET /api/shared` *(access)*

Shares the caller has **received**.

```json
{
  "shares": [
    {
      "id": "...",
      "project": { "id": "...", "name": "nginx-rce" },
      "sharer":  { "id": "...", "username": "alice" },
      "share_mode": "fork-allowed",
      "expires_at": null,
      "created_at": "..."
    }
  ]
}
```

---

## Admin (mounted under `/api/admin`, requires `is_admin = 1`)

| Method + Path | Notes |
|---------------|-------|
| `GET    /api/admin/stats` | counters + 24-hour activity series |
| `GET    /api/admin/users` | full user list (search via `?q=`) |
| `POST   /api/admin/users` | create user (mirrors register but no invite) |
| `PATCH  /api/admin/users/:id` | toggle `is_admin` / `is_active`, change email |
| `DELETE /api/admin/users/:id` | cascade-delete user + machines + projects + observations |
| `POST   /api/admin/users/:id/reset-password` | generate fresh password (response includes plaintext) |
| `GET    /api/admin/invites` | list invite codes |
| `POST   /api/admin/invites` | create invite (`max_uses`, `expires_days`) |
| `DELETE /api/admin/invites/:code` | revoke invite |
| `GET    /api/admin/projects` | global list (filter by `user`, `name`) |
| `GET    /api/admin/observations` | FTS search (`?q=...&user=...&project=...`) |
| `DELETE /api/admin/observations/:id` | hard delete one observation |
| `GET    /api/admin/shares` | global share list |
| `DELETE /api/admin/shares/:id` | force-revoke a share |
| `GET    /api/admin/audit` | audit log (filter by `user`, `action_prefix`) |
| `GET    /api/admin/export/users.csv` | full users dump |
| `GET    /api/admin/export/audit.csv?from=&to=` | audit window |
| `GET    /api/admin/export/observations.csv?user=&project=&from=&to=` | observation export |
| `GET    /api/admin/export/full.db.gz` | `VACUUM INTO` + gzip |
| `GET    /api/admin/export/user/:id.zip` | one-user complete dump (json files) |

Auth for admin endpoints is the **same JWT** as the user-facing surface,
just gated by `users.is_admin = 1 AND is_active = 1`. The admin web
console (`/admin`) reuses the same API via an `HttpOnly` cookie.

Full UI walkthrough: [ADMIN.md](ADMIN.md). Export schemas: same file
under "Export formats".

---

## Rate limiting

Login (`POST /api/auth/login`) is throttled to 5 failures per IP per
15 minutes. Successful logins reset the counter. Other endpoints rely
on the reverse proxy for global rate limits — see
[DEPLOYMENT.md#firewall--network-exposure](DEPLOYMENT.md#firewall--network-exposure).

---

## OpenAPI

`docs/openapi.yaml` is on the roadmap (M9 — see
[Implementation_Plan.md](Implementation_Plan.md)). For now this
document is the source of truth.
