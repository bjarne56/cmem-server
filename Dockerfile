# cmem-server multi-stage Dockerfile
#
# Build:   docker build -t cmem-server .
# Run:     docker run -d -p 127.0.0.1:8080:8080 \
#               -v cmem-data:/var/lib/cmem-server \
#               -v $(pwd)/cmem-server.toml:/etc/cmem-server.toml:ro \
#               cmem-server
#
# Default config bakes in /etc/cmem-server.toml; mount your own to override.

# ─── Builder ────────────────────────────────────────────
FROM rust:1.82-alpine AS builder

# Alpine + musl: rustls instead of native-tls; sqlx ships static SQLite already.
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static

WORKDIR /build

# Cache deps: copy manifests first
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/shared/Cargo.toml crates/shared/Cargo.toml
COPY crates/server/Cargo.toml crates/server/Cargo.toml

# Pre-create empty src to let cargo resolve deps
RUN mkdir -p crates/shared/src crates/server/src \
    && echo "fn main() {}" > crates/server/src/main.rs \
    && echo "" > crates/shared/src/lib.rs \
    && cargo fetch

# Now copy real sources and build
COPY crates ./crates
RUN touch crates/server/src/main.rs crates/shared/src/lib.rs \
    && cargo build --release --bin cmem-server \
    && strip target/release/cmem-server

# ─── Runtime ────────────────────────────────────────────
FROM alpine:3.20 AS runtime

# ca-certificates for outbound TLS (none currently, but future-proof).
# tini for proper PID 1 signal handling.
RUN apk add --no-cache ca-certificates tini sqlite \
    && addgroup -S -g 9501 cmem \
    && adduser  -S -u 9501 -G cmem -D -H -s /sbin/nologin cmem \
    && mkdir -p /var/lib/cmem-server /etc \
    && chown cmem:cmem /var/lib/cmem-server

COPY --from=builder /build/target/release/cmem-server /usr/local/bin/cmem-server
COPY deploy/config/server.toml.example /etc/cmem-server.toml.example

# Ship a default config that points at the volume mount.
RUN cat > /etc/cmem-server.toml.default <<'EOF'
[server]
bind = "0.0.0.0:8080"

[database]
path = "/var/lib/cmem-server/cmem-server.db"

[auth]
jwt_secret = ""
access_token_ttl_secs   = 900
refresh_token_ttl_secs  = 2592000
machine_token_ttl_secs  = 15552000
argon2_memory_kib = 19456
argon2_iterations = 2
argon2_parallelism = 1
require_invite    = false
EOF

USER cmem
WORKDIR /var/lib/cmem-server
EXPOSE 8080
VOLUME ["/var/lib/cmem-server"]

# If /etc/cmem-server.toml is mounted, use it; otherwise fall back to the default.
ENTRYPOINT ["/sbin/tini", "--", "/bin/sh", "-c", \
    "[ -f /etc/cmem-server.toml ] || cp /etc/cmem-server.toml.default /etc/cmem-server.toml; \
     exec /usr/local/bin/cmem-server -c /etc/cmem-server.toml"]

# Healthcheck via the binary itself (no curl dependency).
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -qO- http://127.0.0.1:8080/healthz || exit 1
