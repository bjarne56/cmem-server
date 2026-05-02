# Using cmem-sync

Day-to-day workflows for the **client** half — i.e. the
[claude-mem](https://github.com/thedotmack/claude-mem) CLI and viewer
talking to a running `cmem-server`. Server-side admin lives in
[ADMIN.md](ADMIN.md).

- [First-time setup](#first-time-setup)
- [Pushing observations](#pushing-observations)
- [Pulling on a second machine](#pulling-on-a-second-machine)
- [Project identification](#project-identification)
- [Sharing a project](#sharing-a-project)
- [Forking](#forking)
- [Inspecting state in the viewer](#inspecting-state-in-the-viewer)
- [Working without claude-mem (raw curl)](#working-without-claude-mem-raw-curl)

---

## First-time setup

Install the client (mirror script lives in the
[claude-mem fork](https://github.com/thedotmack/claude-mem)):

```bash
curl -sSL https://raw.githubusercontent.com/<your>/claude-mem/main/install-client.sh \
    | bash -s -- --server https://cmem.example.com
```

It will:

1. Install Node 22+ and Bun (worker runtime).
2. `npm install -g claude-mem` (or your fork).
3. Register the Claude Code hooks.
4. Detect the system language and write
   `~/.claude-mem/settings.json:{"CLAUDE_MEM_MODE": "<lang>"}`.
5. Run `claude-mem sync login --server <URL>` if `--server` was
   supplied.

Manual login:

```bash
claude-mem sync login --server https://cmem.example.com
# username: alice
# password: ********
# machine_name: alice-mac          # auto-detected from navigator.platform
```

This stores a refresh token + machine token in
`~/.claude-mem/sync/tokens.json` (chmod 600).

---

## Pushing observations

Every Claude Code session writes observations to
`~/.claude-mem/memory.db`. Push them up:

```bash
claude-mem sync status         # shows last push, pending count
claude-mem sync push           # one-shot push
claude-mem sync push --watch   # daemon mode (push every 30 s)
```

Or let it run in the background:

```bash
launchctl load ~/Library/LaunchAgents/com.claude-mem.sync.plist   # macOS
systemctl --user enable --now claude-mem-sync.timer               # Linux
```

The CLI batches up to 500 observations per push and reports any
duplicates / errors:

```
Pushing 92 observations to https://cmem.example.com ...
  accepted:    87
  duplicates:   5
  errors:       0
  server_seq:  12 484
  projects:
    nginx-rce  -> 019d1e20-...-bb01
    55ai       -> 019d1e20-...-bb14
```

---

## Pulling on a second machine

```bash
# On your Linux box (different filesystem layout, same person)
claude-mem sync login --server https://cmem.example.com
claude-mem sync pull           # fetch own observations from other machines
```

Pull is incremental. The CLI persists `last_pulled_seq` in
`~/.claude-mem/sync/cmem-sync.db`. To re-fetch everything:

```bash
claude-mem sync pull --reset
```

---

## Project identification

Same logical project on Mac and Linux probably has different absolute
paths:

```
Mac:    /Users/alice/work/nginx-rce
Linux:  /home/alice/projects/nginx-rce
```

The server normalises by `(user_id, project_name)`. As long as the
folder name matches, the two machines write to one project on the
server.

For tighter coupling (or when names differ), pin the project ID by
creating `.cmem-project.toml` in the project root:

```bash
cd ~/work/nginx-rce
claude-mem sync project init
# writes .cmem-project.toml with the server-assigned project_id
git ignore .cmem-project.toml || echo .cmem-project.toml >> .gitignore
```

Once the marker file exists, the project ID is sourced from it on every
push, regardless of folder name.

---

## Sharing a project

```bash
# Read-only to a single user
claude-mem sync share-project nginx-rce --with bob --mode read-only

# Allow Bob to fork
claude-mem sync share-project nginx-rce --with bob --mode fork-allowed

# Auto-copy: new observations stream into Bob's own DB
claude-mem sync share-project nginx-rce --with bob --mode auto-copy

# Public to any logged-in user
claude-mem sync share-project nginx-rce --public --mode read-only

# Anonymous link (with TTL)
claude-mem sync share-project nginx-rce --link --mode read-only --expire 7d
# → https://cmem.example.com/p/abc123XYZ...

# List shares I created
claude-mem sync shared --created

# List shares I received
claude-mem sync shared

# Revoke
claude-mem sync unshare-project nginx-rce --with bob
```

Mode semantics, downgrade rules, and the full state matrix:
[PROJECT_SHARING.md](PROJECT_SHARING.md).

---

## Forking

### Fork an entire project

Only valid when the share mode is `fork-allowed` (or you own the
project and want a private copy):

```bash
claude-mem sync fork-project alice/nginx-rce
# → creates "fork-of-nginx-rce" under your account, copies all
#   observations as derived rows (derived_from + derivation_chain set).
```

Renaming on fork:

```bash
claude-mem sync fork-project alice/nginx-rce --as my-nginx-research
```

### Fork a single observation

Cherry-pick one finding into another of your projects:

```bash
claude-mem sync fork 019d1e2080...abcd --to-project my-research
```

Fork-then-edit becomes a normal own observation; the server tracks the
chain via `derived_from`.

---

## Inspecting state in the viewer

Open `http://127.0.0.1:37701` (the local viewer that ships with
claude-mem). The right-hand drawer shows:

- **Sync status** (server URL, last push / pull, pending counts)
- **Per-project share badges** (read-only / fork-allowed / auto-copy
  + recipient list)
- **Downgrade banner** (when something you receive was demoted; click
  to ack)

The bottom-right floating "Sync" button opens the same form fields as
`claude-mem sync login` for first-time setup.

---

## Working without claude-mem (raw curl)

Every flow above is just REST — see [API.md](API.md). A minimal end-to-end
push:

```bash
SERVER=https://cmem.example.com

# 1. Log in
LOGIN=$(curl -sS "$SERVER/api/auth/login" \
  -H 'content-type: application/json' \
  -d '{"username":"alice","password":"..."}')
ACCESS=$(jq -r .access_token <<<"$LOGIN")

# 2. Register a machine
MACHINE=$(curl -sS "$SERVER/api/machines" \
  -H "authorization: Bearer $ACCESS" \
  -H 'content-type: application/json' \
  -d '{"name":"alice-mac"}')
MTOKEN=$(jq -r .machine_token <<<"$MACHINE")

# 3. Push one observation (JSONL)
cat > /tmp/obs.jsonl <<EOF
{"id":"$(uuidgen | tr A-Z a-z)","timestamp":$(date +%s),"project_marker_id":null,"project_name":"hand-pushed","project_path":"/tmp/hand","content":"hello","obs_type":"observation"}
EOF
curl -sS "$SERVER/api/sync/push" \
  -H "authorization: Bearer $MTOKEN" \
  -H 'content-type: application/x-ndjson' \
  --data-binary @/tmp/obs.jsonl

# 4. Pull
curl -sS "$SERVER/api/sync/pull" \
  -H "authorization: Bearer $MTOKEN" \
  -H 'content-type: application/json' \
  -d '{"since_seq":0,"limit":100,"include_shared":true}' | jq
```

Use `scripts/smoke_auth.sh` and `scripts/smoke_sync.sh` for ready-to-run
end-to-end scripts.

---

## Common pitfalls

| Symptom | Cause | Fix |
|---------|-------|-----|
| `409 username_taken` on register | obvious | pick a different one |
| `401 unauthenticated` on every API | access JWT expired | `claude-mem sync login` again, or call `/api/auth/refresh` |
| pull returns same data forever | `since_seq` not persisted | check `~/.claude-mem/sync/cmem-sync.db` permissions |
| project shows up twice | path normalisation diverged | drop a `.cmem-project.toml` and re-push |
| share mode change has no effect | recipient hasn't pulled | shares apply on next `pull`; force with `claude-mem sync pull --reset` |
| `424 share_revoked` on pull | owner deleted the share | already-forked content stays; re-request the share |

If something keeps misbehaving, check the server logs
(`journalctl -u cmem-server -n 200`) and the audit log
(admin web → Audit Log → filter by your username).
