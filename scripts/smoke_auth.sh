#!/usr/bin/env bash
# 跑通 register → login → refresh → change-password → logout 的 smoke 脚本。
# 假定服务器已经在 http://localhost:8080 运行。
set -euo pipefail

BASE="${BASE:-http://localhost:8080}"
USER="${USER_NAME:-alice-$$}"
PW="${PW:-correct horse battery staple}"
NEW_PW="${NEW_PW:-new-correct-horse-battery-staple}"

say() { printf '\n=== %s ===\n' "$1"; }

say "register $USER"
curl -sS -X POST "$BASE/api/auth/register" \
    -H 'content-type: application/json' \
    -d "$(printf '{"username":"%s","password":"%s"}' "$USER" "$PW")" | tee /tmp/cmem_reg.json
echo

say "login"
LOGIN_RESP=$(curl -sS -X POST "$BASE/api/auth/login" \
    -H 'content-type: application/json' \
    -d "$(printf '{"username":"%s","password":"%s"}' "$USER" "$PW")")
echo "$LOGIN_RESP"
ACCESS=$(printf '%s' "$LOGIN_RESP" | python3 -c 'import sys,json; print(json.load(sys.stdin)["access_token"])')
REFRESH=$(printf '%s' "$LOGIN_RESP" | python3 -c 'import sys,json; print(json.load(sys.stdin)["refresh_token"])')

say "refresh"
REFRESH_RESP=$(curl -sS -X POST "$BASE/api/auth/refresh" \
    -H 'content-type: application/json' \
    -d "$(printf '{"refresh_token":"%s"}' "$REFRESH")")
echo "$REFRESH_RESP"
ACCESS=$(printf '%s' "$REFRESH_RESP" | python3 -c 'import sys,json; print(json.load(sys.stdin)["access_token"])')
REFRESH=$(printf '%s' "$REFRESH_RESP" | python3 -c 'import sys,json; print(json.load(sys.stdin)["refresh_token"])')

say "change-password"
curl -sS -i -X POST "$BASE/api/auth/change-password" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer $ACCESS" \
    -d "$(printf '{"old_password":"%s","new_password":"%s"}' "$PW" "$NEW_PW")"
echo

say "login with new password"
LOGIN_RESP=$(curl -sS -X POST "$BASE/api/auth/login" \
    -H 'content-type: application/json' \
    -d "$(printf '{"username":"%s","password":"%s"}' "$USER" "$NEW_PW")")
echo "$LOGIN_RESP"
ACCESS=$(printf '%s' "$LOGIN_RESP" | python3 -c 'import sys,json; print(json.load(sys.stdin)["access_token"])')
REFRESH=$(printf '%s' "$LOGIN_RESP" | python3 -c 'import sys,json; print(json.load(sys.stdin)["refresh_token"])')

say "logout"
curl -sS -i -X POST "$BASE/api/auth/logout" \
    -H 'content-type: application/json' \
    -H "authorization: Bearer $ACCESS" \
    -d "$(printf '{"refresh_token":"%s"}' "$REFRESH")"
echo

printf '\nDone.\n'
