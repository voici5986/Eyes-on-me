#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$ROOT_DIR/DB"
export EYES_ON_ME_HOST="${EYES_ON_ME_HOST:-127.0.0.1}"
export EYES_ON_ME_PORT="${EYES_ON_ME_PORT:-8787}"
export EYES_ON_ME_DATABASE_URL="${EYES_ON_ME_DATABASE_URL:-sqlite://$ROOT_DIR/DB/eyes-on-me.db}"
export EYES_ON_ME_MEDIA_DIR="${EYES_ON_ME_MEDIA_DIR:-$ROOT_DIR/DB/media}"
exec cargo run -p client-server --manifest-path "$ROOT_DIR/Cargo.toml"
