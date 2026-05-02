# Troubleshooting

If something is broken, this is the place to start. Each section gives
the symptom, the most likely cause, and the fix.

- [Server won't start](#server-wont-start)
- [Service is up but `/healthz` fails](#service-is-up-but-healthz-fails)
- [`401 unauthenticated` on every API](#401-unauthenticated-on-every-api)
- [Login fails with "invalid credentials"](#login-fails-with-invalid-credentials)
- [Push works but pull is empty](#push-works-but-pull-is-empty)
- [Same project shows up twice](#same-project-shows-up-twice)
- [`409 username_taken` even after delete](#409-username_taken-even-after-delete)
- [Admin web returns 403 / redirects to login](#admin-web-returns-403--redirects-to-login)
- [`database is locked` errors](#database-is-locked-errors)
- [Caddy can't reach cmem-server](#caddy-cant-reach-cmem-server)
- [systemd unit fails ProtectSystem](#systemd-unit-fails-protectsystem)
- [Forgotten admin password](#forgotten-admin-password)
- [Reading the audit log](#reading-the-audit-log)
- [Collecting a debug bundle](#collecting-a-debug-bundle)

---

## Server won't start

```bash
sudo systemctl status cmem-server
journalctl -u cmem-server -n 100 --no-pager
```

Common causes:

| Log line | Cause | Fix |
|----------|-------|-----|
| `Permission denied (os error 13)` reading config | mode 0640, group not `cmem` | `sudo chown root:cmem /etc/cmem-server.toml && sudo chmod 0640 ...` |
| `unable to open database file` | data dir missing or wrong owner | `sudo install -d -o cmem -g cmem -m 0750 /var/lib/cmem-server` |
| `bind error: address already in use` | something else holds `:8080` | `sudo lsof -iTCP:8080 -sTCP:LISTEN` and stop it, or change `[server].bind` |
| `failed to apply migrations: ... no such column` | partial restore or schema drift | restore the latest healthy backup |
| `parse server.toml` error | invalid TOML | `cat /etc/cmem-server.toml` and fix the syntax |

---

## Service is up but `/healthz` fails

```bash
curl -v http://127.0.0.1:8080/healthz
```

If you get connection refused but `systemctl status` says active:

```bash
ss -tlnp | grep cmem-server                # what is it actually bound to?
journalctl -u cmem-server -n 50            # recent logs
```

Most often the `[server].bind` line was edited to a non-loopback
address while the firewall blocks it, or you bound to `:8080` but
Caddy proxies `:8081`. Make them match.

---

## `401 unauthenticated` on every API

Three flavours:

1. **Access token expired** (default 15 min). Call `POST
   /api/auth/refresh` with your refresh token. Or in the client:
   `claude-mem sync login`.
2. **JWT secret changed** (manual rotation, restore from a backup with
   different config). Every token issued before the change is dead;
   everyone re-logs-in.
3. **Bearer header missing or malformed.** It must be exactly
   `Authorization: Bearer <token>` — single space, no quotes. `curl
   -v` shows what you actually send.

For machine tokens (sync push/pull), check
`~/.claude-mem/sync/tokens.json` exists and the token starts with
`cmt_`.

---

## Login fails with "invalid credentials"

By design the API returns one identical message regardless of whether
the user is missing, the password is wrong, or the account is
disabled. Verify on the server side:

```bash
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin user list | grep alice
```

Possible findings:

- User does not exist — register or `admin user create`.
- `active=false` — `admin user enable --username alice`.
- User exists, password forgotten — `admin user reset-password
  --username alice`.

If you also locked yourself out by typing the wrong password 5+ times,
the IP throttle kicks in. Wait 15 minutes or restart cmem-server to
clear the in-memory counter.

---

## Push works but pull is empty

Check the cursor. The client persists `last_pulled_seq` in
`~/.claude-mem/sync/cmem-sync.db`:

```bash
sqlite3 ~/.claude-mem/sync/cmem-sync.db \
    "SELECT key, value FROM sync_state;"
```

If `last_pulled_seq` is far ahead of `server_seq`, you've pulled
everything already. Force a re-pull from zero:

```bash
claude-mem sync pull --reset
```

If `server_seq` doesn't grow even after pushes:

```bash
# On server
sudo sqlite3 /var/lib/cmem-server/cmem-server.db \
    "SELECT MAX(server_seq), COUNT(*) FROM observations WHERE deleted_at IS NULL;"
```

Zero observations means push isn't actually landing — check the audit
log for `sync.push` entries, and re-run with `RUST_LOG=cmem_server::sync=debug`.

---

## Same project shows up twice

Almost always project-name divergence between machines. Check:

```bash
sudo sqlite3 /var/lib/cmem-server/cmem-server.db \
    "SELECT id, name, normalized_name FROM projects WHERE user_id = 'YOUR_ID';"
```

If you see two rows that should be one, drop a `.cmem-project.toml`
into the canonical project root with `claude-mem sync project init`,
then `cmem-server admin project merge --src <id> --dst <id>` (when
implemented; for now do it manually with sqlite3 + the audit log
trail).

---

## `409 username_taken` even after delete

`admin user delete` cascades but **soft-deletes** the user row to
preserve audit trail. To free the username:

```bash
sudo sqlite3 /var/lib/cmem-server/cmem-server.db \
    "DELETE FROM users WHERE username = 'alice' AND is_active = 0;"
```

Or rename the soft-deleted row:

```bash
sudo sqlite3 /var/lib/cmem-server/cmem-server.db \
    "UPDATE users SET username = username || '-deleted-' || id WHERE username = 'alice' AND is_active = 0;"
```

---

## Admin web returns 403 / redirects to login

`require_admin` checks **both** `is_admin = 1` and `is_active = 1`. Two
common causes:

1. The user is not flagged admin. Promote:
   ```bash
   sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
       admin user promote --username alice
   ```
2. The cookie expired. Log out and back in at `/admin/login`.

The first admin (right after install) bootstrapping fails sometimes —
fall back to the SQL hatch:

```bash
sudo sqlite3 /var/lib/cmem-server/cmem-server.db \
    "UPDATE users SET is_admin = 1 WHERE username = 'youruser';"
```

---

## `database is locked` errors

WAL mode + a single connection makes this rare. If you see it:

```bash
ls -la /var/lib/cmem-server/cmem-server.db*
```

If `-wal` or `-shm` is unusually large, a previous crash left a stale
checkpoint:

```bash
sudo systemctl stop cmem-server
sudo -u cmem sqlite3 /var/lib/cmem-server/cmem-server.db \
    "PRAGMA wal_checkpoint(TRUNCATE);"
sudo systemctl start cmem-server
```

If that doesn't help, restore the latest backup. Never run two
`cmem-server` processes against the same DB file — `max_connections =
1` only protects against the same process.

---

## Caddy can't reach cmem-server

```bash
sudo systemctl status caddy
sudo caddy validate --config /etc/caddy/Caddyfile
journalctl -u caddy -n 100 --no-pager
```

`502 bad gateway` from Caddy + cmem-server is up = Caddy isn't aimed
at the right port. Check `/etc/caddy/Caddyfile.d/cmem.conf`:

```
reverse_proxy 127.0.0.1:8080
```

vs. what `[server].bind` actually says.

`x509: certificate signed by unknown authority` = Caddy hasn't
finished issuing the cert. Wait 60 s or check `journalctl -u caddy`
for ACME errors (rate-limit, DNS not pointing at your IP, etc.).

---

## systemd unit fails ProtectSystem

```
Failed to set up mount namespacing: Permission denied
```

Some kernels (older Debian / WSL2) lack the namespacing primitives the
unit relies on. Drop the strict mounts:

```bash
sudo systemctl edit cmem-server
```

```ini
[Service]
ProtectSystem=
ProtectHome=
PrivateTmp=
PrivateDevices=
```

Then `sudo systemctl restart cmem-server`. Logs the issue but the
service runs.

---

## Forgotten admin password

```bash
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin user reset-password --username admin
```

If the only admin's account is also locked, promote any working
account first via raw SQL (see [Admin web returns 403](#admin-web-returns-403--redirects-to-login)).

---

## Reading the audit log

```bash
# All events for one user
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin audit --user alice --limit 200

# Just admin actions
sudo sqlite3 /var/lib/cmem-server/cmem-server.db \
    "SELECT created_at, user_id, action, target_id
     FROM audit_log
     WHERE action LIKE 'admin.%'
     ORDER BY created_at DESC LIMIT 50;"

# Failed logins in the last hour
sudo sqlite3 /var/lib/cmem-server/cmem-server.db \
    "SELECT created_at, metadata FROM audit_log
     WHERE action = 'auth.login_failed'
       AND created_at > datetime('now','-1 hour')
     ORDER BY created_at DESC;"
```

Or open the **Audit Log** page in the admin web — same data, plus
filters and CSV export.

---

## Collecting a debug bundle

When opening an issue, attach:

```bash
mkdir -p /tmp/cmem-debug && cd /tmp/cmem-debug
/opt/cmem-server/cmem-server --version       > version.txt
journalctl -u cmem-server -n 500 --no-pager  > server.log
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml admin stats > stats.txt
sudo cat /etc/cmem-server.toml \
    | sed 's/jwt_secret = ".*"/jwt_secret = "<redacted>"/' > config-redacted.toml
ls -la /var/lib/cmem-server > files.txt
tar czf cmem-debug-$(date +%Y%m%d-%H%M%S).tar.gz *.txt *.toml *.log
```

Don't include the SQLite file — it contains observation content.
