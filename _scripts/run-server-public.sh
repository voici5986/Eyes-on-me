#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$ROOT_DIR/DB"

PUBLIC_AGENT_TOKEN="${EYES_ON_ME_AGENT_API_TOKEN:-${AGENT_API_TOKEN:-}}"
PUBLIC_DASHBOARD_TOKEN="${EYES_ON_ME_DASHBOARD_TOKEN:-${DASHBOARD_TOKEN:-}}"
PUBLIC_INTEGRATION_TOKEN="${EYES_ON_ME_INTEGRATION_TOKEN:-}"
if [[ ${#PUBLIC_AGENT_TOKEN} -lt 24 ]]; then
  echo "Public mode requires EYES_ON_ME_AGENT_API_TOKEN (or AGENT_API_TOKEN) with at least 24 characters." >&2
  exit 1
fi
if [[ ${#PUBLIC_DASHBOARD_TOKEN} -lt 24 ]]; then
  echo "Public mode requires EYES_ON_ME_DASHBOARD_TOKEN (or DASHBOARD_TOKEN) with at least 24 characters." >&2
  exit 1
fi
if [[ -n "$PUBLIC_INTEGRATION_TOKEN" && ${#PUBLIC_INTEGRATION_TOKEN} -lt 24 ]]; then
  echo "EYES_ON_ME_INTEGRATION_TOKEN must contain at least 24 characters when integrations are enabled." >&2
  exit 1
fi

export EYES_ON_ME_HOST="${EYES_ON_ME_HOST:-0.0.0.0}"
export EYES_ON_ME_PORT="${EYES_ON_ME_PORT:-8787}"
export EYES_ON_ME_DATABASE_URL="${EYES_ON_ME_DATABASE_URL:-sqlite://$ROOT_DIR/DB/eyes-on-me.db}"
export EYES_ON_ME_MEDIA_DIR="${EYES_ON_ME_MEDIA_DIR:-$ROOT_DIR/DB/media}"
export EYES_ON_ME_AGENT_API_TOKEN="$PUBLIC_AGENT_TOKEN"
export EYES_ON_ME_DASHBOARD_TOKEN="$PUBLIC_DASHBOARD_TOKEN"
exec cargo run -p client-server --manifest-path "$ROOT_DIR/Cargo.toml"
