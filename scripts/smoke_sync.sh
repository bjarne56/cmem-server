#!/usr/bin/env bash
# 跑通 M3+M4+M5 完整路径:
#   1. alice 注册 + 登录 + 注册 mac/linux 两台机器
#   2. 两台机器 push 同名项目(case1)→ pull 看到对方
#   3. alice 把项目 share 给 bob(fork-allowed)→ bob pull 看到
#   4. alice 撤销 share → bob 再 pull,revoked_shares 出现 + shared_observations 空
# 假定服务器在 http://localhost:18080(dev-server.toml)运行。
#
# 用法:bash scripts/smoke_sync.sh

set -euo pipefail

BASE="${BASE:-http://localhost:18080}"
SUFFIX="$$"
ALICE="alice-${SUFFIX}"
BOB="bob-${SUFFIX}"
PW="correct horse battery staple"

say() { printf '\n=== %s ===\n' "$1"; }
jget() {
    # 用 python 抠 json 字段
    python3 -c "import sys,json; print(json.load(sys.stdin)$1)"
}

say "register $ALICE & $BOB"
command curl -sS -X POST "$BASE/api/auth/register" \
    -H 'content-type: application/json' \
    -d "$(printf '{"username":"%s","password":"%s"}' "$ALICE" "$PW")" >/dev/null
command curl -sS -X POST "$BASE/api/auth/register" \
    -H 'content-type: application/json' \
    -d "$(printf '{"username":"%s","password":"%s"}' "$BOB" "$PW")" >/dev/null

say "login alice / bob"
ALICE_LOGIN=$(command curl -sS -X POST "$BASE/api/auth/login" \
    -H 'content-type: application/json' \
    -d "$(printf '{"username":"%s","password":"%s"}' "$ALICE" "$PW")")
ALICE_ACCESS=$(printf '%s' "$ALICE_LOGIN" | jget '["access_token"]')

BOB_LOGIN=$(command curl -sS -X POST "$BASE/api/auth/login" \
    -H 'content-type: application/json' \
    -d "$(printf '{"username":"%s","password":"%s"}' "$BOB" "$PW")")
BOB_ACCESS=$(printf '%s' "$BOB_LOGIN" | jget '["access_token"]')

say "register alice mac & linux + bob mac"
ALICE_MAC=$(command curl -sS -X POST "$BASE/api/machines" \
    -H "authorization: Bearer $ALICE_ACCESS" \
    -H 'content-type: application/json' \
    -d "$(printf '{"name":"alice-mac-%s"}' "$SUFFIX")")
ALICE_MAC_TOKEN=$(printf '%s' "$ALICE_MAC" | jget '["machine_token"]')

ALICE_LINUX=$(command curl -sS -X POST "$BASE/api/machines" \
    -H "authorization: Bearer $ALICE_ACCESS" \
    -H 'content-type: application/json' \
    -d "$(printf '{"name":"alice-linux-%s"}' "$SUFFIX")")
ALICE_LINUX_TOKEN=$(printf '%s' "$ALICE_LINUX" | jget '["machine_token"]')

BOB_MAC=$(command curl -sS -X POST "$BASE/api/machines" \
    -H "authorization: Bearer $BOB_ACCESS" \
    -H 'content-type: application/json' \
    -d "$(printf '{"name":"bob-mac-%s"}' "$SUFFIX")")
BOB_MAC_TOKEN=$(printf '%s' "$BOB_MAC" | jget '["machine_token"]')

say "alice mac push observation in 'nginx-rce'"
PROJECT_NAME="nginx-rce-${SUFFIX}"
PUSH_MAC=$(command curl -sS -X POST "$BASE/api/sync/push" \
    -H "authorization: Bearer $ALICE_MAC_TOKEN" \
    -H 'content-type: application/json' \
    -d "$(cat <<EOF
{"observations":[{
  "id":"01900000-0000-7000-8000-${SUFFIX}001",
  "timestamp":1700000000,
  "project_marker_id":null,
  "project_name":"$PROJECT_NAME",
  "project_path":"/Users/alice/work/$PROJECT_NAME",
  "content":"mac decision",
  "obs_type":"decision",
  "metadata":null,
  "derived_from":null,
  "derivation_chain":null
}]}
EOF
)")
echo "$PUSH_MAC" | python3 -m json.tool
PROJECT_ID=$(printf '%s' "$PUSH_MAC" | jget '["projects_resolved"][0]["project_id"]')
echo "alice project_id = $PROJECT_ID"

say "alice linux push observation in 'NGINX RCE' (规范化合并 case3)"
PUSH_LINUX=$(command curl -sS -X POST "$BASE/api/sync/push" \
    -H "authorization: Bearer $ALICE_LINUX_TOKEN" \
    -H 'content-type: application/json' \
    -d "$(cat <<EOF
{"observations":[{
  "id":"01900000-0000-7000-8000-${SUFFIX}002",
  "timestamp":1700000100,
  "project_marker_id":null,
  "project_name":"NGINX RCE ${SUFFIX}",
  "project_path":"/home/alice/projects/$PROJECT_NAME",
  "content":"linux observation",
  "obs_type":"observation",
  "metadata":null,
  "derived_from":null,
  "derivation_chain":null
}]}
EOF
)")
LINUX_PID=$(printf '%s' "$PUSH_LINUX" | jget '["projects_resolved"][0]["project_id"]')
if [ "$PROJECT_ID" != "$LINUX_PID" ]; then
    echo "FAIL: case 3 normalization should merge but got mac=$PROJECT_ID linux=$LINUX_PID"
    exit 1
fi
echo "OK: case 3 normalization merged into single project_id"

say "alice mac pull (cross-machine sync verification)"
PULL_MAC=$(command curl -sS -X POST "$BASE/api/sync/pull" \
    -H "authorization: Bearer $ALICE_MAC_TOKEN" \
    -H 'content-type: application/json' \
    -d '{"since_seq":0}')
OWN_COUNT=$(printf '%s' "$PULL_MAC" | jget '["own_observations"]|len(_)' 2>/dev/null || \
    printf '%s' "$PULL_MAC" | python3 -c 'import sys,json; print(len(json.load(sys.stdin)["own_observations"]))')
if [ "$OWN_COUNT" != "2" ]; then
    echo "FAIL: alice mac should pull 2 observations (own + linux), got $OWN_COUNT"
    exit 1
fi
echo "OK: alice mac pulls 2 own observations across machines"

say "alice share project to bob (fork-allowed)"
SHARE=$(command curl -sS -X POST "$BASE/api/shares" \
    -H "authorization: Bearer $ALICE_ACCESS" \
    -H 'content-type: application/json' \
    -d "$(cat <<EOF
{
  "project_id":"$PROJECT_ID",
  "target_type":"user",
  "target_username":"$BOB",
  "share_mode":"fork-allowed"
}
EOF
)")
SHARE_ID=$(printf '%s' "$SHARE" | jget '["share"]["id"]')
echo "share_id = $SHARE_ID"

say "bob pull (should see shared_observations from alice)"
BOB_PULL=$(command curl -sS -X POST "$BASE/api/sync/pull" \
    -H "authorization: Bearer $BOB_MAC_TOKEN" \
    -H 'content-type: application/json' \
    -d '{"since_seq":0}')
SHARED_COUNT=$(printf '%s' "$BOB_PULL" | python3 -c 'import sys,json; print(len(json.load(sys.stdin)["shared_observations"]))')
SHARE_MODE=$(printf '%s' "$BOB_PULL" | python3 -c 'import sys,json; obs=json.load(sys.stdin)["shared_observations"]; print(obs[0]["share_mode"] if obs else "")')
if [ "$SHARED_COUNT" -lt 1 ]; then
    echo "FAIL: bob should see >=1 shared observation, got $SHARED_COUNT"
    exit 1
fi
if [ "$SHARE_MODE" != "fork-allowed" ]; then
    echo "FAIL: share_mode should be fork-allowed, got $SHARE_MODE"
    exit 1
fi
echo "OK: bob sees $SHARED_COUNT shared observation(s) with mode=$SHARE_MODE"

say "bob registers a machine (needed to fork)"
BOB_MAC2=$(command curl -sS -X POST "$BASE/api/machines" \
    -H "authorization: Bearer $BOB_ACCESS" \
    -H 'content-type: application/json' \
    -d "$(printf '{"name":"bob-mac2-%s"}' "$SUFFIX")")
BOB_MAC2_TOKEN=$(printf '%s' "$BOB_MAC2" | jget '["machine_token"]')
# 注:bob 之前已经注册过 BOB_MAC,fork 会优先用 last_seen 最近的机器。

say "bob forks alice's project (invariant #6)"
FORK=$(command curl -sS -X POST "$BASE/api/projects/$PROJECT_ID/fork" \
    -H "authorization: Bearer $BOB_ACCESS" \
    -H 'content-type: application/json' \
    -d '{}')
COPIED=$(printf '%s' "$FORK" | python3 -c 'import sys,json; print(json.load(sys.stdin)["copied_observations"])')
FORK_PID=$(printf '%s' "$FORK" | python3 -c 'import sys,json; print(json.load(sys.stdin)["project"]["id"])')
if [ "$COPIED" -lt 2 ]; then
    echo "FAIL: fork should copy >=2 observations (alice has 2), got $COPIED"
    exit 1
fi
echo "OK: bob forked $COPIED observations into project $FORK_PID"

say "bob pull → own_observations 含 fork 副本(带 derived_from)"
BOB_PULL_FORK=$(command curl -sS -X POST "$BASE/api/sync/pull" \
    -H "authorization: Bearer $BOB_MAC_TOKEN" \
    -H 'content-type: application/json' \
    -d '{"since_seq":0}')
BOB_OWN=$(printf '%s' "$BOB_PULL_FORK" | python3 -c 'import sys,json; print(len(json.load(sys.stdin)["own_observations"]))')
HAS_DERIVED=$(printf '%s' "$BOB_PULL_FORK" | python3 -c 'import sys,json; obs=json.load(sys.stdin)["own_observations"]; print(all(o.get("derived_from") for o in obs) if obs else False)')
if [ "$BOB_OWN" -lt 2 ] || [ "$HAS_DERIVED" != "True" ]; then
    echo "FAIL: bob should have $COPIED forked own_observations all with derived_from, got $BOB_OWN derived=$HAS_DERIVED"
    exit 1
fi
echo "OK: bob has $BOB_OWN own observations all with derived_from"

say "alice revokes the share"
command curl -sS -X DELETE "$BASE/api/shares/$SHARE_ID" \
    -H "authorization: Bearer $ALICE_ACCESS"

say "bob pull again (shared_observations should be empty, revoked_shares populated)"
BOB_PULL2=$(command curl -sS -X POST "$BASE/api/sync/pull" \
    -H "authorization: Bearer $BOB_MAC_TOKEN" \
    -H 'content-type: application/json' \
    -d '{"since_seq":0}')
SHARED2=$(printf '%s' "$BOB_PULL2" | python3 -c 'import sys,json; print(len(json.load(sys.stdin)["shared_observations"]))')
REVOKED2=$(printf '%s' "$BOB_PULL2" | python3 -c 'import sys,json; print(len(json.load(sys.stdin)["revoked_shares"]))')
if [ "$SHARED2" != "0" ]; then
    echo "FAIL: after revoke, shared_observations should be empty, got $SHARED2"
    exit 1
fi
if [ "$REVOKED2" -lt 1 ]; then
    echo "FAIL: revoked_shares should contain the revoked project, got $REVOKED2"
    exit 1
fi
echo "OK: revoke wiped shared view; revoked_shares contains $REVOKED2 entry"

say "alice's own observations still intact (invariant #4)"
ALICE_PULL=$(command curl -sS -X POST "$BASE/api/sync/pull" \
    -H "authorization: Bearer $ALICE_MAC_TOKEN" \
    -H 'content-type: application/json' \
    -d '{"since_seq":0}')
ALICE_OWN=$(printf '%s' "$ALICE_PULL" | python3 -c 'import sys,json; print(len(json.load(sys.stdin)["own_observations"]))')
if [ "$ALICE_OWN" != "2" ]; then
    echo "FAIL: alice own observations broken after revoke (expected 2, got $ALICE_OWN)"
    exit 1
fi
echo "OK: alice still has $ALICE_OWN own observations"

say "bob's forked observations survive the revoke (invariant #4)"
BOB_PULL_AFTER_REVOKE=$(command curl -sS -X POST "$BASE/api/sync/pull" \
    -H "authorization: Bearer $BOB_MAC_TOKEN" \
    -H 'content-type: application/json' \
    -d '{"since_seq":0}')
BOB_OWN_AFTER=$(printf '%s' "$BOB_PULL_AFTER_REVOKE" | python3 -c 'import sys,json; print(len(json.load(sys.stdin)["own_observations"]))')
if [ "$BOB_OWN_AFTER" -lt 2 ]; then
    echo "FAIL: bob's forked copies must persist after revoke, got $BOB_OWN_AFTER"
    exit 1
fi
echo "OK: bob still has $BOB_OWN_AFTER forked observations after revoke"

printf '\n*** smoke_sync.sh: ALL CHECKS PASSED ***\n'
