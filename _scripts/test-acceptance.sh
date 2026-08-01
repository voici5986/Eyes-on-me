#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER_BIN="${EYES_ON_ME_ACCEPTANCE_SERVER_BIN:-$ROOT_DIR/target/debug/client-server}"
PORT="${EYES_ON_ME_ACCEPTANCE_PORT:-18787}"
BASE_URL="http://127.0.0.1:$PORT"
AGENT_TOKEN='acceptance-agent-token-20260729'
DASHBOARD_TOKEN='acceptance-dashboard-token-20260729'
INTEGRATION_TOKEN='acceptance-integration-token-20260729'
ACCEPTANCE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/eyes-on-me-acceptance.XXXXXX")"
COOKIE_JAR="$ACCEPTANCE_DIR/dashboard.cookies"
SERVER_LOG="$ACCEPTANCE_DIR/server.log"
SERVER_PID=''

cleanup() {
  local status=$?
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  if [[ $status -ne 0 && -f "$SERVER_LOG" ]]; then
    printf '\nServer log:\n' >&2
    tail -80 "$SERVER_LOG" >&2 || true
  fi
  case "$ACCEPTANCE_DIR" in
    */eyes-on-me-acceptance.*) rm -rf -- "$ACCEPTANCE_DIR" ;;
  esac
}
trap cleanup EXIT INT TERM

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  exit 1
}

pass() {
  printf 'PASS: %s\n' "$1"
}

expect_jq() {
  local json="$1"
  local filter="$2"
  local label="$3"
  printf '%s' "$json" | jq -e "$filter" >/dev/null || fail "$label"
  pass "$label"
}

for command in curl jq base64; do
  command -v "$command" >/dev/null || fail "required command is unavailable: $command"
done
[[ -x "$SERVER_BIN" ]] || fail "server binary is missing; run cargo build --workspace first"
if curl -fsS --max-time 1 "$BASE_URL/health" >/dev/null 2>&1; then
  fail "acceptance port $PORT is already serving HTTP"
fi

mkdir -p "$ACCEPTANCE_DIR/media"
EYES_ON_ME_HOST=127.0.0.1 \
EYES_ON_ME_PORT="$PORT" \
EYES_ON_ME_DATABASE_URL="sqlite://$ACCEPTANCE_DIR/acceptance.db" \
EYES_ON_ME_MEDIA_DIR="$ACCEPTANCE_DIR/media" \
EYES_ON_ME_AGENT_API_TOKEN="$AGENT_TOKEN" \
EYES_ON_ME_DASHBOARD_TOKEN="$DASHBOARD_TOKEN" \
EYES_ON_ME_INTEGRATION_TOKEN="$INTEGRATION_TOKEN" \
EYES_ON_ME_OCR_COMMAND=off \
EYES_ON_ME_WEB_DIST="$ROOT_DIR/web/dist" \
"$SERVER_BIN" >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!

for _ in {1..100}; do
  if curl -fsS --max-time 1 "$BASE_URL/health" >/dev/null 2>&1; then
    break
  fi
  kill -0 "$SERVER_PID" 2>/dev/null || fail 'server exited during startup'
  sleep 0.05
done

health="$(curl -fsS "$BASE_URL/health")"
expect_jq "$health" '.ok == true' 'health endpoint'
[[ -s "$ACCEPTANCE_DIR/acceptance.db" ]] || fail 'SQLite migrations/database creation'
pass 'SQLite migrations/database creation'

if date -u -v-1M '+%Y-%m-%dT%H:%M:%SZ' >/dev/null 2>&1; then
  DAY="$(date -v-1d '+%Y-%m-%d')"
  WEEKDAY="$(date -v-1d '+%u')"
  BASE_EPOCH="$(date -j -f '%Y-%m-%d %H:%M:%S' "$DAY 12:00:00" '+%s')"
  TS0="$(date -u -r "$BASE_EPOCH" '+%Y-%m-%dT%H:%M:%SZ')"
  TS1="$(date -u -r "$((BASE_EPOCH + 60))" '+%Y-%m-%dT%H:%M:%SZ')"
  TS2="$(date -u -r "$((BASE_EPOCH + 120))" '+%Y-%m-%dT%H:%M:%SZ')"
  TS3="$(date -u -r "$((BASE_EPOCH + 180))" '+%Y-%m-%dT%H:%M:%SZ')"
  TS_END="$(date -u -r "$((BASE_EPOCH + 300))" '+%Y-%m-%dT%H:%M:%SZ')"
else
  DAY="$(date -d yesterday '+%Y-%m-%d')"
  WEEKDAY="$(date -d yesterday '+%u')"
  BASE_EPOCH="$(date -d "$DAY 12:00:00" '+%s')"
  TS0="$(date -u -d "@$BASE_EPOCH" '+%Y-%m-%dT%H:%M:%SZ')"
  TS1="$(date -u -d "@$((BASE_EPOCH + 60))" '+%Y-%m-%dT%H:%M:%SZ')"
  TS2="$(date -u -d "@$((BASE_EPOCH + 120))" '+%Y-%m-%dT%H:%M:%SZ')"
  TS3="$(date -u -d "@$((BASE_EPOCH + 180))" '+%Y-%m-%dT%H:%M:%SZ')"
  TS_END="$(date -u -d "@$((BASE_EPOCH + 300))" '+%Y-%m-%dT%H:%M:%SZ')"
fi

anonymous_status="$(curl -sS -o /dev/null -w '%{http_code}' "$BASE_URL/api/devices")"
[[ "$anonymous_status" == '401' ]] || fail 'dashboard protected endpoint rejects anonymous request'
pass 'dashboard authentication gate'

login="$(curl -fsS -c "$COOKIE_JAR" -H 'content-type: application/json' \
  --data "$(jq -nc --arg token "$DASHBOARD_TOKEN" '{token:$token}')" \
  "$BASE_URL/api/auth/login")"
expect_jq "$login" '.authenticated == true and .authRequired == true' 'dashboard login and session'

post_event() {
  local event_id="$1"
  local timestamp="$2"
  local app_id="$3"
  local app_name="$4"
  local title="$5"
  local page_title="$6"
  local url="$7"
  local domain="$8"
  local browser_json='null'
  if [[ -n "$domain" ]]; then
    browser_json="$(jq -nc \
      --arg name "$app_name" \
      --arg pageTitle "$page_title" \
      --arg url "$url" \
      --arg domain "$domain" \
      '{family:"chromium",name:$name,pageTitle:$pageTitle,url:$url,domain:$domain,source:"accessibility-native",confidence:0.98}')"
  fi
  jq -nc \
    --arg eventId "$event_id" \
    --arg ts "$timestamp" \
    --arg appId "$app_id" \
    --arg appName "$app_name" \
    --arg title "$title" \
    --argjson browser "$browser_json" \
    '{eventId:$eventId,ts:$ts,deviceId:"acceptance-device",agentName:"acceptance-agent",platform:"macos",kind:"foreground_changed",app:{id:$appId,name:$appName,title:$title,pid:4242},windowTitle:$title,browser:$browser,presence:"active",source:"macos-native"}' |
    curl -fsS -H "authorization: Bearer $AGENT_TOKEN" -H 'content-type: application/json' \
      --data-binary @- "$BASE_URL/api/agent/activity" >/dev/null
}

post_event 'e-browser-a' "$TS0" 'com.google.Chrome' 'Google Chrome' \
  'Architecture - TODO follow up' 'Architecture - TODO follow up' \
  'https://docs.example.com/architecture' 'docs.example.com'
post_event 'e-browser-b' "$TS1" 'com.google.Chrome' 'Google Chrome' \
  'Metrics Dashboard' 'Metrics Dashboard' \
  'https://metrics.example.com/dashboard' 'metrics.example.com'
post_event 'e-terminal-a' "$TS2" 'com.apple.Terminal' 'Terminal' \
  'cargo test | Eyes_on_me' '' '' ''
post_event 'e-terminal-b' "$TS3" 'com.apple.Terminal' 'Terminal' \
  'server logs | Eyes_on_me' '' '' ''

status_payload="$(jq -nc --arg ts "$TS3" \
  '{ts:$ts,deviceId:"acceptance-device",agentName:"acceptance-agent",platform:"macos",statusText:"active",source:"macos-native"}')"
curl -fsS -H "authorization: Bearer $AGENT_TOKEN" -H 'content-type: application/json' \
  --data "$status_payload" "$BASE_URL/api/agent/status" >/dev/null
pass 'browser and Terminal Tab event ingestion'

devices="$(curl -fsS -b "$COOKIE_JAR" "$BASE_URL/api/devices")"
expect_jq "$devices" '.devices | length == 1 and .[0].device.windowTitle == "server logs | Eyes_on_me"' 'device summary current Tab context'

timeline="$(curl -fsS -b "$COOKIE_JAR" --get \
  --data-urlencode "start=$TS0" --data-urlencode "end=$TS_END" \
  --data-urlencode 'deviceId=acceptance-device' --data-urlencode 'limit=2' \
  "$BASE_URL/api/timeline")"
expect_jq "$timeline" '.total == 4 and (.items | length) == 2 and .limit == 2 and .offset == 0' 'timeline RFC3339 range and pagination'

timeline_page_two="$(curl -fsS -b "$COOKIE_JAR" --get \
  --data-urlencode "start=$TS0" --data-urlencode "end=$TS_END" \
  --data-urlencode 'deviceId=acceptance-device' --data-urlencode 'limit=2' \
  --data-urlencode 'offset=2' "$BASE_URL/api/timeline")"
expect_jq "$timeline_page_two" '.total == 4 and (.items | length) == 2 and .offset == 2' 'timeline second page'

browser_filter="$(curl -fsS -b "$COOKIE_JAR" --get \
  --data-urlencode "start=$TS0" --data-urlencode "end=$TS_END" \
  --data-urlencode 'app=Google Chrome' "$BASE_URL/api/timeline")"
expect_jq "$browser_filter" '.total == 2 and ([.items[].activity.app.name] | unique) == ["Google Chrome"]' 'timeline app filter'

domain_filter="$(curl -fsS -b "$COOKIE_JAR" --get \
  --data-urlencode "start=$TS0" --data-urlencode "end=$TS_END" \
  --data-urlencode 'domain=docs.example.com' "$BASE_URL/api/timeline")"
expect_jq "$domain_filter" '.total == 1 and .items[0].activity.browser.pageTitle == "Architecture - TODO follow up"' 'timeline domain and browser Tab detail'

text_filter="$(curl -fsS -b "$COOKIE_JAR" --get \
  --data-urlencode "start=$TS0" --data-urlencode "end=$TS_END" \
  --data-urlencode 'q=TODO follow up' "$BASE_URL/api/timeline")"
expect_jq "$text_filter" '.total == 1' 'timeline text filter'

analysis="$(curl -fsS -b "$COOKIE_JAR" "$BASE_URL/api/devices/acceptance-device/analysis?range=1w")"
expect_jq "$analysis" '(.appUsage | length) == 2 and ([.appUsage[] | select(.label=="Google Chrome")][0].windows | length) == 2 and ([.appUsage[] | select(.label=="Terminal")][0].windows | length) == 2' 'single app block and per-Tab drilldown'

if base64 --help 2>&1 | grep -q -- '--decode'; then
  printf '%s' 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=' | base64 --decode >"$ACCEPTANCE_DIR/pixel.png"
else
  printf '%s' 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=' | base64 -D >"$ACCEPTANCE_DIR/pixel.png"
fi
screenshot="$(curl -fsS -H "authorization: Bearer $AGENT_TOKEN" -H 'content-type: image/png' \
  --data-binary "@$ACCEPTANCE_DIR/pixel.png" "$BASE_URL/api/agent/screenshots/e-browser-a")"
expect_jq "$screenshot" '.eventId == "e-browser-a" and .ocrStatus == "unavailable" and .byteSize > 0' 'screenshot storage with OCR disabled'
screenshot_list="$(curl -fsS -b "$COOKIE_JAR" "$BASE_URL/api/screenshots?limit=10")"
expect_jq "$screenshot_list" '.screenshots | length == 1 and .[0].eventId == "e-browser-a"' 'screenshot listing'
media_status="$(curl -fsS -b "$COOKIE_JAR" "$BASE_URL/api/media/status")"
expect_jq "$media_status" '.screenshotCount == 1 and .pendingOcr == 0' 'media status'
remote_status="$(curl -fsS -b "$COOKIE_JAR" "$BASE_URL/api/media/remote")"
expect_jq "$remote_status" '.configured == false and .pending == 0 and .failed == 0' 'remote mirror unconfigured status'

default_settings="$(curl -fsS -b "$COOKIE_JAR" "$BASE_URL/api/settings/review")"
expect_jq "$default_settings" '(.workSchedule | length) == 5' 'default review settings'
updated_settings="$(printf '%s' "$default_settings" | jq -c --argjson weekday "$WEEKDAY" '
  .categoryRules = [{id:"deep-work",name:"Deep Work",color:"#2F855A",target:"domain",pattern:"docs.example.com",priority:100}] |
  .workSchedule = [
    {id:"morning",weekday:$weekday,startMinute:540,endMinute:720},
    {id:"afternoon",weekday:$weekday,startMinute:780,endMinute:1080}
  ] |
  .reportPreferences.pinnedBlocks = ["overview"] |
  .reportPreferences.hiddenBlocks = ["screenshots"] |
  .reportPreferences.blockOrder = ["overview","applications","websites","screenshots"]')"
saved_settings="$(curl -fsS -b "$COOKIE_JAR" -H 'content-type: application/json' \
  -X PUT --data-binary "$updated_settings" "$BASE_URL/api/settings/review")"
expect_jq "$saved_settings" '(.categoryRules | length) == 1 and (.workSchedule | length) == 2 and .reportPreferences.hiddenBlocks == ["screenshots"]' 'category, multi-segment schedule, and report preferences'

category_filter="$(curl -fsS -b "$COOKIE_JAR" --get \
  --data-urlencode "start=$TS0" --data-urlencode "end=$TS_END" \
  --data-urlencode 'category=deep-work' "$BASE_URL/api/timeline")"
expect_jq "$category_filter" '.total == 1 and .items[0].categoryLabel == "Deep Work"' 'historical category backfill'

settings_backup="$(curl -fsS -b "$COOKIE_JAR" "$BASE_URL/api/settings/review/export")"
expect_jq "$settings_backup" '.format == "eyes-on-me-review-settings" and .version == 1 and (.settings.workSchedule | length) == 2' 'settings JSON backup'
restored_settings="$(curl -fsS -b "$COOKIE_JAR" -H 'content-type: application/json' \
  --data-binary "$settings_backup" "$BASE_URL/api/settings/review/import")"
expect_jq "$restored_settings" '(.categoryRules | length) == 1 and (.workSchedule | length) == 2' 'settings JSON restore'

sessions="$(curl -fsS -b "$COOKIE_JAR" --get --data-urlencode "date=$DAY" \
  --data-urlencode 'deviceId=acceptance-device' "$BASE_URL/api/sessions")"
expect_jq "$sessions" '(.sessions | length) == 1 and .sessions[0].totalTrackedMs > 0 and (.sessions[0].potentialTodos | length) >= 1' 'work sessions and TODO clues'

default_control="$(curl -fsS -H "authorization: Bearer $AGENT_TOKEN" "$BASE_URL/api/agent/control/acceptance-device")"
expect_jq "$default_control" '.recording.desiredEnabled == true and .recording.appliedEnabled == null and .recording.revision == 0' 'default device recording state'
paused="$(curl -fsS -b "$COOKIE_JAR" -H 'content-type: application/json' -X PUT \
  --data '{"enabled":false}' "$BASE_URL/api/devices/acceptance-device/recording")"
expect_jq "$paused" '.desiredEnabled == false and .revision == 1 and .acknowledgedAt == null' 'dashboard recording pause'
revision="$(printf '%s' "$paused" | jq -r '.revision')"
polled_control="$(curl -fsS -H "authorization: Bearer $AGENT_TOKEN" "$BASE_URL/api/agent/control/acceptance-device")"
expect_jq "$polled_control" '.recording.desiredEnabled == false and .recording.revision == 1' 'Agent control poll'
acknowledged="$(curl -fsS -H "authorization: Bearer $AGENT_TOKEN" -H 'content-type: application/json' \
  --data "$(jq -nc --argjson revision "$revision" '{enabled:false,revision:$revision}')" \
  "$BASE_URL/api/agent/control/acceptance-device")"
expect_jq "$acknowledged" '.recording.appliedEnabled == false and .recording.acknowledgedAt != null' 'Agent recording acknowledgement'

report="$(curl -fsS -b "$COOKIE_JAR" -X POST "$BASE_URL/api/reports/$DAY/generate?useAi=false")"
expect_jq "$report" '.generationMode == "deterministic" and (.content | startswith("# ")) and (.content | contains("截图线索") | not)' 'deterministic report and block preferences'
report_history="$(curl -fsS -b "$COOKIE_JAR" --get --data-urlencode "start=$DAY" \
  --data-urlencode "end=$DAY" "$BASE_URL/api/reports")"
expect_jq "$report_history" 'length == 1 and .[0].generationMode == "deterministic"' 'report history'
report_export="$(curl -fsS -b "$COOKIE_JAR" --get --data-urlencode "start=$DAY" \
  --data-urlencode "end=$DAY" "$BASE_URL/api/reports/export")"
[[ "$report_export" == \#* ]] || fail 'report range export'
pass 'report range export'

assistant_stream="$(curl -fsS -b "$COOKIE_JAR" -H 'content-type: application/json' \
  --data "$(jq -nc --arg prompt "总结 $DAY 的工作，并列出待办" '{prompt:$prompt,useAi:false}')" \
  "$BASE_URL/api/assistant/stream")"
printf '%s\n' "$assistant_stream" | jq -s -e 'any(.type == "token") and any(.type == "done")' >/dev/null || fail 'assistant NDJSON stream'
pass 'assistant NDJSON stream'
conversation_id="$(printf '%s\n' "$assistant_stream" | jq -r 'select(.type == "done") | .reply.conversation.id')"
conversation="$(curl -fsS -b "$COOKIE_JAR" "$BASE_URL/api/assistant/conversations/$conversation_id")"
expect_jq "$conversation" '(.messages | length) == 2 and .messages[0].role == "user" and .messages[1].role == "assistant"' 'assistant conversation history'

mcp_initialize="$(curl -fsS -H "authorization: Bearer $INTEGRATION_TOKEN" -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' "$BASE_URL/mcp")"
expect_jq "$mcp_initialize" '.result.serverInfo.name == "eyes-on-me"' 'MCP initialize'
mcp_tools="$(curl -fsS -H "authorization: Bearer $INTEGRATION_TOKEN" -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' "$BASE_URL/mcp")"
expect_jq "$mcp_tools" '(.result.tools | map(.name) | index("timeline")) != null' 'MCP tools/list'
mcp_call="$(curl -fsS -H "authorization: Bearer $INTEGRATION_TOKEN" -H 'content-type: application/json' \
  --data "$(jq -nc --arg date "$DAY" '{jsonrpc:"2.0",id:3,method:"tools/call",params:{name:"timeline",arguments:{date:$date,deviceId:"acceptance-device",limit:10}}}')" \
  "$BASE_URL/mcp")"
expect_jq "$mcp_call" '.result.isError == false and .result.structuredContent.total == 4' 'MCP tools/call'

deletion="$(curl -fsS -b "$COOKIE_JAR" -H 'content-type: application/json' \
  --data "$(jq -nc --arg date "$DAY" '{date:$date,app:"Google Chrome"}')" \
  "$BASE_URL/api/activities/bulk-delete")"
expect_jq "$deletion" '.deletedActivities == 2 and .deletedScreenshots == 1' 'transactional app bulk deletion'
report_status="$(curl -sS -b "$COOKIE_JAR" -o /dev/null -w '%{http_code}' "$BASE_URL/api/reports/$DAY")"
[[ "$report_status" == '404' ]] || fail 'bulk deletion invalidates generated report'
pass 'bulk deletion invalidates generated report'
remaining="$(curl -fsS -b "$COOKIE_JAR" --get --data-urlencode "start=$TS0" \
  --data-urlencode "end=$TS_END" "$BASE_URL/api/timeline")"
expect_jq "$remaining" '.total == 2 and ([.items[].activity.app.name] | unique) == ["Terminal"]' 'bulk deletion keeps unmatched activities'
media_after_delete="$(curl -fsS -b "$COOKIE_JAR" "$BASE_URL/api/media/status")"
expect_jq "$media_after_delete" '.screenshotCount == 0' 'bulk deletion removes bound screenshot'

pass 'all isolated runtime acceptance checks'
