#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ ! -d "$ROOT_DIR/web/node_modules" ]; then
  (
    cd "$ROOT_DIR/web"
    pnpm install
  )
fi

cd "$ROOT_DIR/web"
exec pnpm dev
