#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$ROOT_DIR/DB"
export AMI_OKAY_HOST="${AMI_OKAY_HOST:-0.0.0.0}"
export AMI_OKAY_PORT="${AMI_OKAY_PORT:-8787}"
export AMI_OKAY_DATABASE_URL="${AMI_OKAY_DATABASE_URL:-sqlite://$ROOT_DIR/DB/eyes-on-me.db}"
exec cargo run -p client-server --manifest-path "$ROOT_DIR/Cargo.toml"
