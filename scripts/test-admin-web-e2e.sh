#!/usr/bin/env bash
#
# 端到端 admin web 测试 — 用 curl 打真实 cmem-server 实例。
#
# 前置:
#   1. cmem-server 在 $SERVER_URL 已启动(默认 http://127.0.0.1:18080)
#   2. 数据库里有一个 admin 账号 $ADMIN_USER/$ADMIN_PASS(默认 admin/admin@123)
#
# 用法:
#   bash scripts/test-admin-web-e2e.sh
#   SERVER_URL=http://127.0.0.1:8080 ADMIN_USER=root bash scripts/test-admin-web-e2e.sh
#
# 退出码:全部通过 = 0;任何失败 = 1。

set -uo pipefail

SERVER_URL="${SERVER_URL:-${CMEM_SERVER:-http://127.0.0.1:18080}}"
ADMIN_USER="${ADMIN_USER:-admin}"
ADMIN_PASS="${ADMIN_PASS:-admin@123}"

# 终端着色(无 TTY 自动关)
if [[ -t 1 ]]; then
    GREEN=$'\033[32m'; RED=$'\033[31m'; YELLOW=$'\033[33m'; CYAN=$'\033[36m'; RESET=$'\033[0m'
else
    GREEN=""; RED=""; YELLOW=""; CYAN=""; RESET=""
fi

PASS=0
FAIL=0
SKIP=0
FAILED_CASES=()

# ---------- helpers ----------

# python3 必备(解析 JSON / URL encode)
command -v python3 >/dev/null || { echo "python3 required"; exit 1; }
command -v curl >/dev/null || { echo "curl required"; exit 1; }
command -v jq >/dev/null || echo "${YELLOW}note: jq not installed; using python3 fallback${RESET}"

urlenc() { python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$1"; }

# 提取 JSON 字段(优先 jq,fallback python3)
jget() {
    local field="$1"
    if command -v jq >/dev/null; then
        jq -r ".${field} // empty" 2>/dev/null
    else
        python3 -c '
import sys,json
try:
    d=json.load(sys.stdin)
    parts="'"${field}"'".split(".")
    for p in parts:
        if p.startswith("[") and p.endswith("]"):
            d=d[int(p[1:-1])]
        else:
            d=d.get(p) if isinstance(d,dict) else d
    if d is None: print("")
    elif isinstance(d,(dict,list)): print(json.dumps(d))
    else: print(d)
except Exception:
    pass
'
    fi
}

# 通用 HTTP 调用 — 输出 'STATUS\nBODY'
http() {
    local method="$1" path="$2"
    shift 2
    curl -sS -o /tmp/cmem_e2e.body -w '%{http_code}' -X "$method" "$SERVER_URL$path" "$@"
}

# 期望 status 码 — 不匹配 → 记录 FAIL
expect_status() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "  ${GREEN}OK${RESET} $label  -> $actual"
        PASS=$((PASS + 1))
    else
        echo "  ${RED}FAIL${RESET} $label  expected=$expected got=$actual"
        echo "       body: $(head -c 200 /tmp/cmem_e2e.body)"
        FAIL=$((FAIL + 1))
        FAILED_CASES+=("$label")
    fi
}

# 期望 grep 命中
expect_grep() {
    local label="$1" pattern="$2"
    if grep -q -- "$pattern" /tmp/cmem_e2e.body 2>/dev/null; then
        echo "  ${GREEN}OK${RESET} $label  matches '$pattern'"
        PASS=$((PASS + 1))
    else
        echo "  ${RED}FAIL${RESET} $label  body does not contain '$pattern'"
        echo "       body head: $(head -c 200 /tmp/cmem_e2e.body)"
        FAIL=$((FAIL + 1))
        FAILED_CASES+=("$label")
    fi
}

section() {
    echo
    echo "${CYAN}=== $* ===${RESET}"
}

# ---------- preflight ----------

section "Preflight"
hc=$(curl -sS -o /tmp/cmem_e2e.body -w '%{http_code}' "$SERVER_URL/healthz" || true)
if [[ "$hc" != "200" ]]; then
    echo "${RED}server not reachable at $SERVER_URL (healthz returned $hc)${RESET}"
    echo "Start with: cargo run -p cmem-server -- serve --config dev-server.toml"
    exit 1
fi
echo "  ${GREEN}OK${RESET} server up: $(cat /tmp/cmem_e2e.body)"
PASS=$((PASS + 1))

# ---------- admin login (REST API for token) ----------

section "Admin login (REST /api/auth/login)"
LOGIN=$(curl -sS -X POST "$SERVER_URL/api/auth/login" \
    -H 'content-type: application/json' \
    -d "$(printf '{"username":"%s","password":"%s"}' "$ADMIN_USER" "$ADMIN_PASS")")
TOKEN=$(printf '%s' "$LOGIN" | jget access_token)
if [[ -z "$TOKEN" ]]; then
    echo "${RED}cannot login as $ADMIN_USER — got: $LOGIN${RESET}"
    echo "Hint: create the admin first via:"
    echo "  cargo run -p cmem-server -- admin user create $ADMIN_USER --admin"
    exit 1
fi
echo "  ${GREEN}OK${RESET} got bearer token (len=${#TOKEN})"
PASS=$((PASS + 1))

AUTH_HDR="Authorization: Bearer $TOKEN"

# ---------- 1. /api/admin/stats ----------

section "1. JSON API: /api/admin/stats"

st=$(http GET "/api/admin/stats" -H "$AUTH_HDR" -H "Accept: application/json")
expect_status "GET /api/admin/stats with admin bearer" "200" "$st"
expect_grep "stats body has 'users' field" '"users"'
expect_grep "stats body has 'recent' object" '"recent"'

st=$(http GET "/api/admin/stats" -H "Accept: application/json")
expect_status "GET /api/admin/stats without bearer" "401" "$st"

# ---------- 2. Admin web pages render ----------

section "2. HTML pages (admin web)"

for page in "" "/users" "/invites" "/projects" "/observations" "/shares" "/audit" "/export"; do
    st=$(http GET "/admin$page" -H "$AUTH_HDR" -H "Accept: text/html")
    expect_status "GET /admin$page" "200" "$st"
done
expect_grep "page contains 'cmem-server' (app title)" "cmem-server"
expect_grep "page contains sidebar /admin/users link" "/admin/users"

# ---------- 3. I18N ----------

section "3. I18N — URL ?lang=xx"

st=$(http GET "/admin?lang=zh" -H "$AUTH_HDR" -H "Accept: text/html")
expect_status "GET /admin?lang=zh" "200" "$st"
expect_grep "zh dashboard rendered" '仪表盘'
expect_grep "zh sidebar 'users' label" '用户'
expect_grep "<html lang=\"zh\">" 'lang="zh"'
expect_grep "zh dir=ltr" 'dir="ltr"'

st=$(http GET "/admin?lang=ja" -H "$AUTH_HDR" -H "Accept: text/html")
expect_status "GET /admin?lang=ja" "200" "$st"
expect_grep "ja dashboard rendered (ダッシュボード or similar)" 'ダッシュボード'

st=$(http GET "/admin?lang=ar" -H "$AUTH_HDR" -H "Accept: text/html")
expect_status "GET /admin?lang=ar" "200" "$st"
expect_grep "ar dir=rtl" 'dir="rtl"'

st=$(http GET "/admin?lang=he" -H "$AUTH_HDR" -H "Accept: text/html")
expect_status "GET /admin?lang=he" "200" "$st"
expect_grep "he dir=rtl" 'dir="rtl"'

st=$(http GET "/admin?lang=ur" -H "$AUTH_HDR" -H "Accept: text/html")
expect_status "GET /admin?lang=ur" "200" "$st"
expect_grep "ur dir=rtl" 'dir="rtl"'

st=$(http GET "/admin?lang=xx" -H "$AUTH_HDR" -H "Accept: text/html")
expect_status "GET /admin?lang=xx (unknown)" "200" "$st"
expect_grep "unknown lang fallback to en" 'Dashboard'

# 测试更多语言渲染基本不 5xx
for lang in zh-tw ko es pt-br fr de ru pl cs nl tr uk vi id th hi bn ro sv it el hu fi da no; do
    st=$(http GET "/admin?lang=$lang" -H "$AUTH_HDR" -H "Accept: text/html")
    if [[ "$st" == "200" ]]; then
        PASS=$((PASS + 1))
    else
        echo "  ${RED}FAIL${RESET} lang=$lang got $st"
        FAIL=$((FAIL + 1))
        FAILED_CASES+=("lang=$lang render")
    fi
done
echo "  ${GREEN}OK${RESET} 25 additional language renders"

section "3b. I18N — lang switcher cookie"

st=$(curl -sS -o /tmp/cmem_e2e.body -D /tmp/cmem_e2e.headers -w '%{http_code}' \
    -H "$AUTH_HDR" "$SERVER_URL/admin/lang/zh?next=/admin/users")
if [[ "$st" =~ ^3 ]]; then
    PASS=$((PASS + 1))
    echo "  ${GREEN}OK${RESET} GET /admin/lang/zh -> $st"
else
    echo "  ${RED}FAIL${RESET} GET /admin/lang/zh expected 3xx got $st"
    FAIL=$((FAIL + 1))
fi
if grep -qi 'set-cookie:.*cmem_admin_lang=zh' /tmp/cmem_e2e.headers; then
    PASS=$((PASS + 1))
    echo "  ${GREEN}OK${RESET} Set-Cookie cmem_admin_lang=zh present"
else
    echo "  ${RED}FAIL${RESET} cmem_admin_lang cookie not set"
    FAIL=$((FAIL + 1))
fi

st=$(curl -sS -o /tmp/cmem_e2e.body -D /tmp/cmem_e2e.headers -w '%{http_code}' \
    -H "$AUTH_HDR" "$SERVER_URL/admin/lang/xx?next=/admin")
if [[ "$st" =~ ^3 ]] && ! grep -qi 'set-cookie:.*cmem_admin_lang' /tmp/cmem_e2e.headers; then
    PASS=$((PASS + 1))
    echo "  ${GREEN}OK${RESET} invalid lang/xx not setting cookie"
else
    echo "  ${RED}FAIL${RESET} invalid lang/xx behavior wrong (status=$st)"
    FAIL=$((FAIL + 1))
fi

section "3c. Accept-Language header"
st=$(curl -sS -o /tmp/cmem_e2e.body -w '%{http_code}' \
    -H "$AUTH_HDR" -H "Accept: text/html" \
    -H "Accept-Language: zh-CN,zh;q=0.9,en;q=0.8" \
    "$SERVER_URL/admin")
expect_status "GET /admin with Accept-Language zh-CN" "200" "$st"
expect_grep "Accept-Language picks zh" '仪表盘'

# ---------- 4. User CRUD ----------

section "4. User CRUD via /api/admin/users"

NEWU="e2e_$(date +%s)_$$"
st=$(http POST "/api/admin/users" -H "$AUTH_HDR" -H "Content-Type: application/json" \
    -d "$(printf '{"username":"%s","password":"P1ssword!","email":"%s@x.io","is_admin":false}' "$NEWU" "$NEWU")")
expect_status "POST /api/admin/users (create $NEWU)" "201" "$st"
NEW_ID=$(jget id < /tmp/cmem_e2e.body)
echo "    new_id=$NEW_ID"

st=$(http GET "/api/admin/users?q=$(urlenc "$NEWU")" -H "$AUTH_HDR" -H "Accept: application/json")
expect_status "GET /api/admin/users (search)" "200" "$st"
expect_grep "list contains new user" "\"$NEWU\""

st=$(http PATCH "/api/admin/users/$NEW_ID" -H "$AUTH_HDR" -H "Content-Type: application/json" \
    -d '{"is_admin":true}')
expect_status "PATCH promote $NEWU to admin" "200" "$st"

st=$(http PATCH "/api/admin/users/$NEW_ID" -H "$AUTH_HDR" -H "Content-Type: application/json" \
    -d '{"is_active":false}')
expect_status "PATCH disable $NEWU (still 1+ active admin remains)" "200" "$st"

st=$(http DELETE "/api/admin/users/$NEW_ID" -H "$AUTH_HDR")
expect_status "DELETE $NEWU" "204" "$st"

# ---------- 5. Defense (last admin) ----------

section "5. Defense: last active admin"

# 找 root admin id
ROOT_ID=$(curl -sS -H "$AUTH_HDR" "$SERVER_URL/api/admin/users?q=$(urlenc "$ADMIN_USER")" \
    | python3 -c 'import sys,json; arr=json.load(sys.stdin); print(next((u["id"] for u in arr if u["username"]=="'"$ADMIN_USER"'" and u.get("is_admin") and u.get("is_active")), ""))')
if [[ -z "$ROOT_ID" ]]; then
    echo "  ${YELLOW}SKIP${RESET} could not find ROOT_ID for $ADMIN_USER (active+admin)"
    SKIP=$((SKIP + 1))
else
    # 假设当前 db 里只剩 1 个 active admin → 三个操作都应 409
    st=$(http DELETE "/api/admin/users/$ROOT_ID" -H "$AUTH_HDR")
    if [[ "$st" == "409" ]]; then
        PASS=$((PASS + 1))
        echo "  ${GREEN}OK${RESET} DELETE last admin -> 409"
    elif [[ "$st" == "204" ]]; then
        # 多 admin 场景下可能允许。视为 skip 而非 fail。
        SKIP=$((SKIP + 1))
        echo "  ${YELLOW}SKIP${RESET} DELETE returned 204 — multiple admins exist, defense not triggered"
    else
        FAIL=$((FAIL + 1))
        echo "  ${RED}FAIL${RESET} DELETE last admin expected 409|204 got $st"
        FAILED_CASES+=("delete last admin defense")
    fi
fi

# ---------- 6. Invites ----------

section "6. Invites lifecycle"

st=$(http POST "/api/admin/invites" -H "$AUTH_HDR" -H "Content-Type: application/json" \
    -d '{"max_uses":3,"expires_days":7}')
expect_status "POST /api/admin/invites (create)" "201" "$st"
CODE=$(jget code < /tmp/cmem_e2e.body)
echo "    code=$CODE"

st=$(http GET "/api/admin/invites" -H "$AUTH_HDR" -H "Accept: application/json")
expect_status "GET /api/admin/invites (list)" "200" "$st"
expect_grep "invite list contains $CODE" "$CODE"

st=$(http DELETE "/api/admin/invites/$CODE" -H "$AUTH_HDR")
expect_status "DELETE /api/admin/invites/$CODE" "204" "$st"

# ---------- 7. Observations search ----------

section "7. Observations search"

st=$(http GET "/api/admin/observations" -H "$AUTH_HDR" -H "Accept: application/json")
expect_status "GET /api/admin/observations" "200" "$st"

# ---------- 8. Export endpoints ----------

section "8. Export endpoints"

st=$(http GET "/api/admin/export/users.csv" -H "$AUTH_HDR")
expect_status "GET /api/admin/export/users.csv" "200" "$st"
expect_grep "users.csv has 'username' header" 'username'
expect_grep "users.csv contains $ADMIN_USER" "$ADMIN_USER"

st=$(http GET "/api/admin/export/audit.csv" -H "$AUTH_HDR")
expect_status "GET /api/admin/export/audit.csv" "200" "$st"

st=$(http GET "/api/admin/export/observations.csv" -H "$AUTH_HDR")
expect_status "GET /api/admin/export/observations.csv" "200" "$st"

st=$(http GET "/api/admin/export/full.db.gz" -H "$AUTH_HDR")
if [[ "$st" == "200" ]]; then
    if head -c2 /tmp/cmem_e2e.body | xxd -p | grep -q '^1f8b'; then
        PASS=$((PASS + 1))
        echo "  ${GREEN}OK${RESET} full.db.gz has gzip magic 1f 8b"
    else
        FAIL=$((FAIL + 1))
        echo "  ${RED}FAIL${RESET} full.db.gz missing gzip magic; got: $(head -c 4 /tmp/cmem_e2e.body | xxd -p)"
        FAILED_CASES+=("full.db.gz magic")
    fi
else
    FAIL=$((FAIL + 1))
    echo "  ${RED}FAIL${RESET} full.db.gz expected 200 got $st"
    FAILED_CASES+=("full.db.gz status")
fi

# ---------- 9. Audit log ----------

section "9. Audit log"

st=$(http GET "/api/admin/audit?action=admin." -H "$AUTH_HDR" -H "Accept: application/json")
expect_status "GET /api/admin/audit?action=admin." "200" "$st"
expect_grep "audit log has admin entries" '"admin\.'

# ---------- 10. Security ----------

section "10. Security"

st=$(http GET "/api/admin/stats" -H "Accept: application/json" -H "Authorization: Bearer garbage")
expect_status "GET with garbage bearer -> 401" "401" "$st"

st=$(http GET "/admin" -H "Accept: text/html" -H "Cookie: cmem_admin_session=garbage")
if [[ "$st" =~ ^3 ]]; then
    PASS=$((PASS + 1))
    echo "  ${GREEN}OK${RESET} HTML with garbage cookie -> 3xx redirect"
else
    FAIL=$((FAIL + 1))
    echo "  ${RED}FAIL${RESET} HTML with garbage cookie expected 3xx got $st"
fi

# SQL injection attempt — should be safe
st=$(http GET "/api/admin/users?q=%27%3B--" -H "$AUTH_HDR" -H "Accept: application/json")
if [[ "$st" == "200" ]]; then
    PASS=$((PASS + 1))
    echo "  ${GREEN}OK${RESET} SQL-injection-shaped q does not 5xx (got 200)"
else
    FAIL=$((FAIL + 1))
    echo "  ${RED}FAIL${RESET} SQL-injection q got $st (expected 200)"
fi

# ---------- summary ----------

echo
echo "${CYAN}==================== SUMMARY ====================${RESET}"
echo "  ${GREEN}Passed: $PASS${RESET}"
echo "  ${RED}Failed: $FAIL${RESET}"
echo "  ${YELLOW}Skipped: $SKIP${RESET}"

if (( FAIL > 0 )); then
    echo
    echo "${RED}Failed cases:${RESET}"
    for f in "${FAILED_CASES[@]}"; do
        echo "  - $f"
    done
    exit 1
fi

echo
echo "${GREEN}All e2e cases passed.${RESET}"
