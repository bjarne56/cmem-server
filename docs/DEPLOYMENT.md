# Deploying cmem-server in production

This document covers everything beyond `install-server.sh`: VPS sizing,
Docker, Kubernetes, backups, monitoring, firewalling, log rotation,
TLS, and reverse-proxy choices.

- [Sizing & topology](#sizing--topology)
- [VPS deployment (recommended)](#vps-deployment-recommended)
- [Reverse proxy](#reverse-proxy)
  - [Caddy (default)](#caddy-default)
  - [nginx](#nginx)
  - [Cloudflare Tunnel](#cloudflare-tunnel)
- [Docker](#docker)
- [Kubernetes](#kubernetes)
- [Backups](#backups)
- [Monitoring](#monitoring)
- [Log rotation](#log-rotation)
- [Firewall + network exposure](#firewall--network-exposure)
- [Disaster recovery](#disaster-recovery)
- [Capacity planning](#capacity-planning)

---

## Sizing & topology

`cmem-server` is intentionally tiny: one Rust binary (~10 MB stripped),
one SQLite file. The bottleneck is **disk I/O** (SQLite WAL fsync), not
CPU or memory.

| Users | Observations / day | Recommended VPS | DB growth |
|-------|---------------------|-----------------|-----------|
| 1     | < 1k                | $5/mo, 1 CPU, 1 GB, SSD | < 100 MB / month |
| 10    | < 10k               | $5–$10/mo               | ~ 1 GB / month   |
| 50    | < 100k              | $10–$20/mo, 2 CPU, 2 GB | ~ 5 GB / month   |
| 200+  | > 1M                | dedicated SSD, 4 GB     | tune WAL checkpoint, snapshot to object storage |

There is **no horizontal scaling story** — SQLite writes serialise on
one file. If you outgrow this, the right move is to replace
`db::pool` with an external Postgres backend (a 2-day port). Don't do
that until you actually outgrow SQLite.

### Recommended topology

```
                 Internet
                    |
              443 (TLS)
                    |
+-------------------v--------------------+
|             Caddy / nginx              |
|   - Let's Encrypt cert                 |
|   - HSTS, gzip, X-Forwarded-For        |
+-------------------+--------------------+
                    |
        loopback :8080 (no TLS)
                    |
+-------------------v--------------------+
|              cmem-server               |
|   axum + sqlx + SQLite (WAL)           |
+-------------------+--------------------+
                    |
+-------------------v--------------------+
|       /var/lib/cmem-server/            |
|        cmem-server.db (+ WAL/SHM)      |
+----------------------------------------+
```

cmem-server is **never** exposed directly to the public internet:

- it has no built-in TLS termination
- it has no IP allow-list
- it relies on the reverse proxy for Real-IP propagation

Configure `bind = "127.0.0.1:8080"` and let Caddy / nginx handle
everything inbound.

---

## VPS deployment (recommended)

Tested on a $5/mo Hetzner CX11 / DigitalOcean droplet.

```bash
# 1. SSH in as root (or a sudoer)
ssh root@cmem.example.com

# 2. Clone + install + configure HTTPS in one shot
git clone https://github.com/bjarne/cmem-server
cd cmem-server
sudo ./scripts/install-server.sh --domain cmem.example.com

# 3. Confirm
curl https://cmem.example.com/healthz
# → { "status": "ok", "version": "0.1.0" }

# 4. Reset the bootstrap admin password!
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin user reset-password --username admin
```

If you want invite-only registration (recommended for any
team-shared deployment), edit `/etc/cmem-server.toml`:

```toml
[auth]
require_invite = true
```

Then `systemctl restart cmem-server` and create invites from the
admin CLI:

```bash
sudo -u cmem /opt/cmem-server/cmem-server -c /etc/cmem-server.toml \
    admin invite create --max-uses 1 --expires-days 7
```

---

## Reverse proxy

### Caddy (default)

`install-server.sh --domain` drops a snippet at
`/etc/caddy/Caddyfile.d/cmem.conf`:

```caddyfile
cmem.example.com {
    reverse_proxy 127.0.0.1:8080 {
        header_up X-Real-IP {remote_host}
        header_up X-Forwarded-For {remote_host}
        header_up X-Forwarded-Proto {scheme}
    }
    encode gzip zstd
    log {
        output file /var/log/caddy/cmem-server.log
        format json
    }
}
```

Caddy auto-issues and renews TLS certs from Let's Encrypt. No further
action needed.

### nginx

```nginx
server {
    listen 443 ssl http2;
    listen [::]:443 ssl http2;
    server_name cmem.example.com;

    ssl_certificate     /etc/letsencrypt/live/cmem.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/cmem.example.com/privkey.pem;
    ssl_protocols       TLSv1.2 TLSv1.3;

    add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;
    add_header X-Content-Type-Options "nosniff" always;

    client_max_body_size 32M;            # sync push payloads can be large

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host              $host;
        proxy_set_header X-Real-IP         $remote_addr;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout                 300s;
        proxy_buffering                    off;
    }
}

server {
    listen 80;
    listen [::]:80;
    server_name cmem.example.com;
    return 301 https://$host$request_uri;
}
```

Use `certbot --nginx -d cmem.example.com` to wire up Let's Encrypt.

### Cloudflare Tunnel

If you don't want to open ports at all:

```bash
# Install cloudflared, then
cloudflared tunnel create cmem
cloudflared tunnel route dns cmem cmem.example.com
cat > ~/.cloudflared/config.yml <<EOF
tunnel: cmem
credentials-file: /root/.cloudflared/<id>.json
ingress:
  - hostname: cmem.example.com
    service: http://localhost:8080
  - service: http_status:404
EOF
sudo cloudflared service install
```

cmem-server stays bound to `127.0.0.1:8080`; Cloudflare handles TLS at
the edge.

---

## Docker

### Single-container quick start

```bash
docker run -d --name cmem-server \
    -p 127.0.0.1:8080:8080 \
    -v cmem-data:/var/lib/cmem-server \
    -e CMEM_BIND=0.0.0.0:8080 \
    --restart unless-stopped \
    ghcr.io/bjarne/cmem-server:latest
```

### docker-compose with Caddy

`deploy/docker/docker-compose.yml` (see also `Dockerfile`):

```yaml
services:
  cmem-server:
    image: ghcr.io/bjarne/cmem-server:latest
    container_name: cmem-server
    restart: unless-stopped
    expose: ["8080"]
    volumes:
      - cmem-data:/var/lib/cmem-server
      - ./cmem-server.toml:/etc/cmem-server.toml:ro

  caddy:
    image: caddy:2-alpine
    container_name: cmem-caddy
    restart: unless-stopped
    ports: ["80:80", "443:443", "443:443/udp"]
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy-data:/data
      - caddy-config:/config

volumes:
  cmem-data:
  caddy-data:
  caddy-config:
```

`Caddyfile`:

```caddyfile
cmem.example.com {
    reverse_proxy cmem-server:8080
}
```

```bash
docker compose up -d
docker compose logs -f
```

---

## Kubernetes

Not officially supported but trivial:

```yaml
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: cmem-server
spec:
  serviceName: cmem-server
  replicas: 1                     # SQLite -> always 1
  selector:
    matchLabels: { app: cmem-server }
  template:
    metadata:
      labels: { app: cmem-server }
    spec:
      containers:
        - name: cmem-server
          image: ghcr.io/bjarne/cmem-server:latest
          ports: [{ containerPort: 8080 }]
          volumeMounts:
            - { name: data, mountPath: /var/lib/cmem-server }
            - { name: cfg,  mountPath: /etc/cmem-server.toml, subPath: cmem-server.toml }
          readinessProbe:
            httpGet: { path: /healthz, port: 8080 }
          resources:
            requests: { cpu: 50m,  memory: 64Mi }
            limits:   { cpu: 500m, memory: 256Mi }
      volumes:
        - name: cfg
          configMap: { name: cmem-server-config }
  volumeClaimTemplates:
    - metadata: { name: data }
      spec:
        accessModes: [ReadWriteOnce]
        resources: { requests: { storage: 5Gi } }
```

Use `Recreate` strategy (not RollingUpdate) — two pods cannot share the
SQLite file.

---

## Backups

The entire state is `/var/lib/cmem-server/cmem-server.db` (plus its
`-wal` / `-shm`). Backup options:

### 1. `VACUUM INTO` (safe online snapshot)

```bash
#!/bin/bash
# /etc/cron.daily/cmem-backup
set -euo pipefail
DST=/var/backups/cmem
mkdir -p "$DST"
sqlite3 /var/lib/cmem-server/cmem-server.db \
    "VACUUM INTO '$DST/cmem-$(date +%Y%m%d).db'"
gzip -9 "$DST/cmem-$(date +%Y%m%d).db"
find "$DST" -name 'cmem-*.db.gz' -mtime +30 -delete
```

`VACUUM INTO` is atomic; it works while cmem-server is serving traffic.

### 2. Admin web export

`Admin -> Export -> Full DB (.db.gz)` writes the same `VACUUM INTO`
dump and serves it over HTTPS. Useful for ad-hoc grabs.

### 3. Off-site replication

Push the daily gzip to S3 / B2 / R2 / Backblaze:

```bash
DST=/var/backups/cmem
aws s3 cp "$DST/cmem-$(date +%Y%m%d).db.gz" s3://my-backups/cmem/
# or
rclone copy "$DST/cmem-$(date +%Y%m%d).db.gz" r2:my-backups/cmem/
```

### Restore

```bash
sudo systemctl stop cmem-server
sudo gunzip -c /path/to/backup.db.gz \
    > /var/lib/cmem-server/cmem-server.db
sudo chown cmem:cmem /var/lib/cmem-server/cmem-server.db
# Delete WAL / SHM so SQLite reopens cleanly
sudo rm -f /var/lib/cmem-server/cmem-server.db-{wal,shm}
sudo systemctl start cmem-server
curl -s http://127.0.0.1:8080/healthz
```

---

## Monitoring

### Liveness / readiness

```bash
curl -fsS http://127.0.0.1:8080/healthz
```

`/healthz` returns 200 + `{"status":"ok","version":"0.1.0"}` while the
DB pool is healthy. Use it for k8s probes, uptime checks, etc.

### Prometheus

The server logs structured JSON via `tracing-subscriber`. The simplest
metrics path is to scrape the systemd unit's restart count and DB file
size:

```bash
# node_exporter textfile collector example (/var/lib/node_exporter/textfile/cmem.prom)
echo "cmem_db_bytes $(stat -c%s /var/lib/cmem-server/cmem-server.db)"
```

Native Prometheus endpoint is on the roadmap (see
[Implementation_Plan.md](Implementation_Plan.md)).

### Logs

`journalctl -u cmem-server` (Linux) or `tail -f
/usr/local/var/cmem-server/cmem-server.log` (macOS).

Set log level via env var:

```bash
sudo systemctl edit cmem-server
# add:
[Service]
Environment=RUST_LOG=cmem_server=debug,tower_http=debug
```

---

## Log rotation

Linux with systemd uses the journal — no rotation needed unless you
hit the journal size cap (`journalctl --disk-usage`).

If you redirect to a file (e.g. `>> /var/log/cmem-server.log`), drop a
`/etc/logrotate.d/cmem-server`:

```
/var/log/cmem-server.log {
    daily
    rotate 14
    compress
    missingok
    notifempty
    sharedscripts
    postrotate
        systemctl reload cmem-server > /dev/null 2>&1 || true
    endscript
}
```

Caddy logs are rotated by Caddy itself if you set `roll_size` /
`roll_keep` (see `deploy/caddy/Caddyfile.example`).

---

## Firewall + network exposure

Default cmem-server bind is `127.0.0.1:8080`. Only the reverse proxy
should reach it. With Caddy / nginx on the same host, **no port other
than 80 / 443 needs to be open**.

UFW example:

```bash
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw allow 22/tcp
sudo ufw allow 80,443/tcp
sudo ufw enable
```

If you must run cmem-server directly on the public internet (no proxy),
**at minimum** add an IP allow-list at the firewall layer or in front
of it. The server has no built-in DDoS or rate limiting beyond the
auth-attempt counter.

---

## Disaster recovery

1. **DB corruption** — restore the latest `VACUUM INTO` dump (above).
2. **Lost JWT secret** — every issued access / refresh / machine token
   is invalidated. Users must re-login; machines must re-register.
   Generate a new one and restart:
   ```bash
   sudo sed -i "s/^jwt_secret = .*/jwt_secret = \"$(openssl rand -hex 32)\"/" /etc/cmem-server.toml
   sudo systemctl restart cmem-server
   ```
3. **Lost admin password** — promote any other user, or reset via
   sqlite3:
   ```bash
   sqlite3 /var/lib/cmem-server/cmem-server.db \
       "UPDATE users SET is_admin = 1 WHERE username = 'youruser';"
   ```
4. **Compromised host** — assume every JWT and machine token is
   compromised. Rotate `jwt_secret`, force everyone to re-login,
   review `audit_log`.

---

## Capacity planning

A rough rule, measured on a $5 Hetzner droplet:

- write throughput (push): ~ 800 observations / second sustained
- read throughput (pull):  ~ 4 000 observations / second sustained
- per-observation row size: ~ 1.2 KB on disk (depends on content)
- argon2id login cost (default RFC 9106 params): ~ 100 ms / call

For team-scale (< 50 active users): default config is fine. For larger
deployments, the first knob to turn is `[auth].argon2_memory_kib`
(lower it to keep login latency < 200 ms) — at the cost of password
hardness. Stick with RFC 9106 defaults unless you understand the
tradeoff.

---

For day-to-day admin tasks (creating users, exporting CSV, etc.) see
[ADMIN.md](ADMIN.md). For client-side flows (push / pull / share) see
[USAGE.md](USAGE.md).
