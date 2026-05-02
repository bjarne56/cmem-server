# Security

This document covers cmem-server's threat model, the controls already
implemented, what you must do as an operator, and how to disclose a
vulnerability.

- [Threat model](#threat-model)
- [Built-in controls](#built-in-controls)
- [Hardening checklist](#hardening-checklist)
- [Cryptography](#cryptography)
- [Token lifecycle](#token-lifecycle)
- [JWT secret rotation](#jwt-secret-rotation)
- [HTTPS](#https)
- [IP propagation](#ip-propagation)
- [Rate limiting](#rate-limiting)
- [Audit logging](#audit-logging)
- [Reporting a vulnerability](#reporting-a-vulnerability)

---

## Threat model

cmem-server is a self-hosted, single-tenant or small-team service. The
realistic adversaries:

| Adversary | Goal | In scope? |
|-----------|------|-----------|
| Network attacker (TLS unconfigured) | sniff JWTs / passwords | yes — must run behind HTTPS |
| Brute-force login | guess weak passwords | yes — argon2id + login throttle |
| Lost / stolen device | use machine token long after | yes — `DELETE /api/machines/:id` |
| Malicious authenticated user | escalate to admin | yes — `is_admin` flag, audit log |
| Other authenticated user | read someone else's project | yes — owner / share permission checks |
| Operator (root on the host) | read DB | **out of scope** (encrypt at rest at OS layer) |
| Supply-chain attack on dependencies | inject backdoor | partial — `Cargo.lock` checked in, deps reviewed quarterly |
| Side-channel timing on argon2 | extract password material | out of scope (argon2id constant-time) |
| DDoS | crash the service | out of scope (use Cloudflare / fail2ban / proxy rate limit) |

**Out-of-scope is not "we don't care"** — for those threats, the
mitigation lives outside the application boundary. SECURITY.md flags
each one explicitly so you know where to look.

---

## Built-in controls

- **Passwords**: argon2id, RFC 9106 defaults (19 MiB / 2 iter / 1
  thread). Hashes only — plaintext never written to disk or logs.
- **JWT secret**: 256-bit random; auto-generated on first start if the
  config field is empty.
- **Tokens stored as SHA-256**: refresh tokens and machine tokens are
  hashed before insert; the database never holds the plaintext.
- **Machine token format**: `cmt_<32-char nanoid>` — collision
  probability is `<10^-30` even at billions of machines.
- **Sqlx compile-time SQL checks**: every query is validated against
  the schema at `cargo build` time. No runtime string interpolation
  into SQL.
- **`unwrap()` ban** outside test code, enforced by code review +
  `cargo clippy --workspace -- -D warnings`.
- **Body size limits**: 8 MiB JSON, 32 MiB sync push.
- **CORS off by default** — same-origin only; admin web is on the same
  origin as the API.
- **Soft delete with global filtering**: every observation /
  project_share query joins `deleted_at IS NULL` / `revoked_at IS
  NULL`.
- **Admin gate**: `require_admin` middleware checks `is_admin = 1 AND
  is_active = 1` on every `/api/admin/*` and protected `/admin/*` hit.
- **Audit log**: every write goes into `audit_log` with user_id,
  machine_id (if any), action, and target. Exported as CSV from the
  admin web.

---

## Hardening checklist

Tick these before exposing the service to the public:

- [ ] Bind to `127.0.0.1:8080`, never `0.0.0.0` directly to the
      internet.
- [ ] Reverse-proxy HTTPS termination via Caddy / nginx with
      Let's Encrypt or a managed cert.
- [ ] HSTS enabled at the proxy (`max-age=31536000;
      includeSubDomains`). The Caddy template does this for `/admin/*`.
- [ ] `[auth].require_invite = true` if anyone other than you can
      reach `/api/auth/register`.
- [ ] Rotate the bootstrap admin password immediately.
- [ ] Deploy `fail2ban` or equivalent for SSH (cmem-server has
      built-in login throttle but the OS still needs hardening).
- [ ] DB file permissions: `0640 cmem:cmem` (default with
      install-server.sh).
- [ ] Config file permissions: `0640 root:cmem` (the JWT secret is in
      there).
- [ ] Daily backup to off-host storage
      ([DEPLOYMENT.md#backups](DEPLOYMENT.md#backups)).
- [ ] `journalctl --rotate` configured so the audit log doesn't fill
      `/var/log`.
- [ ] Monitor `audit_log` for `auth.login_failed` spikes and
      `admin.user_create` / `admin.user_promote` events.

---

## Cryptography

| Where | Algorithm | Notes |
|-------|-----------|-------|
| Password hash | argon2id | RFC 9106 defaults; configurable in `[auth]` |
| JWT signing | HS256 | 256-bit secret in `[auth].jwt_secret` |
| Refresh token | random 32-byte → SHA-256 in DB | rotated on every refresh |
| Machine token | `cmt_<32 nanoid>` → SHA-256 in DB | TTL configurable |
| Share link token | 32-char nanoid | stored as plaintext (anonymous lookup) |

All randomness comes from `OsRng` (`getrandom` syscall on Linux,
`SecRandomCopyBytes` on macOS).

Asymmetric crypto is intentionally absent: the service is single-server,
all traffic is bearer-token over TLS, and the operational complexity of
key management isn't justified at this scale.

---

## Token lifecycle

```
register / login
      |
      v
+-----+--------------------------+
| access JWT, TTL 15 min default | --> sent on every authenticated call
+--------------------------------+
| refresh JWT, TTL 30 days       | --> only used to mint new access JWTs
+--------------------------------+
            |
            v
       refresh
            |
            +--> new access + new refresh; old refresh revoked
            |
            v
         logout
            |
            +--> refresh revoked, access expires naturally

machine register
      |
      v
machine token cmt_<32>  (TTL 180 days default)
      |
      +--> use for /api/sync/{push,pull} without round-tripping login
      +--> revoke via DELETE /api/machines/:id
```

`change-password` invalidates **all** refresh tokens for the user.
`logout` invalidates only the supplied refresh token. Access tokens
are stateless (HS256-signed); they are only invalidated by waiting out
their TTL or by rotating `jwt_secret`.

---

## JWT secret rotation

**When to rotate**:

- on initial install (auto)
- if the host is compromised
- if the secret is committed anywhere by accident
- on a routine schedule (yearly is more than enough at this scale)

**How**:

```bash
sudo sed -i "s/^jwt_secret = .*/jwt_secret = \"$(openssl rand -hex 32)\"/" /etc/cmem-server.toml
sudo systemctl restart cmem-server
```

Effect: every active session is invalidated. Users must `claude-mem
sync login` again; machines must re-register.

You can introduce a graceful transition by running both old and new
secrets briefly through a forked deployment, but the simple "rotate +
restart + everyone re-logs-in" path is what the codebase supports
today.

---

## HTTPS

cmem-server has **no built-in TLS**. Every operator must front it with
a TLS-terminating reverse proxy. The included Caddy template
(`deploy/caddy/Caddyfile.example`) is the lowest-friction option —
Caddy auto-issues, auto-renews, and auto-redirects HTTP → HTTPS.

If you absolutely need TLS in-process (e.g. air-gapped network with no
proxy), patch `crates/server/src/server.rs` to use `axum_server::bind_rustls`.
The codebase doesn't include this by default because it adds a heavy
crypto dependency for a feature 99 % of users don't need.

---

## IP propagation

For audit_log to record the **real** client IP (not the reverse
proxy's), the proxy must add `X-Forwarded-For` and cmem-server must
trust it. The included Caddy snippet does the first half:

```caddy
header_up X-Real-IP {remote_host}
header_up X-Forwarded-For {remote_host}
```

cmem-server reads `X-Real-IP` first, falls back to
`X-Forwarded-For` (last entry), and finally to `ConnectInfo`. **Only
trust these headers when the request comes from your reverse proxy** —
if cmem-server is reachable from the internet directly, anyone can
spoof them. Bind to `127.0.0.1` to make spoofing impossible.

---

## Rate limiting

- Login attempts: 5 failures per IP per 15 minutes (in-memory; survives
  process restart only by re-counting from the audit log on the next
  failure).
- Other endpoints: not rate-limited at the application layer. Use the
  reverse proxy:

```caddy
{
    rate_limit {
        zone api 10r/s burst=20
    }
}
```

For nginx, see `limit_req_zone`. For Cloudflare, use Rate Limiting
Rules in the dashboard.

---

## Audit logging

Every write hits `audit_log`. Examples of recorded actions:

```
auth.register                      auth.login                auth.login_failed
auth.refresh                       auth.logout               auth.change_password
auth.password_reset                machine.create            machine.revoke
project.create                     project.update            project.delete
project.fork                       share.create              share.update
share.revoke                       sync.push                 sync.pull
admin.user_create                  admin.user_update         admin.user_delete
admin.user_promote                 admin.user_demote         admin.password_reset
admin.invite_create                admin.invite_revoke       admin.export
```

Inspect from the admin web (Audit tab) or the CLI:

```bash
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin audit --user alice --limit 100
```

The table is small (one row per write); no rotation is necessary at
the scale this server is built for. Export to long-term storage
quarterly via `admin export audit.csv`.

---

## Reporting a vulnerability

Email **security@bjarne.example.com** (or open a draft GitHub Security
Advisory at <https://github.com/bjarne/cmem-server/security/advisories>).
Please include:

- A reproducer (curl commands, code snippet, or PoC repo)
- Affected versions / commit hash
- Impact assessment

We will:

1. Acknowledge within 72 hours.
2. Triage within 7 days.
3. Push a patched release within 30 days for high severity, 90 days
   for medium / low.
4. Credit you in the release notes (unless you prefer otherwise).

This project does not currently run a paid bug bounty.
