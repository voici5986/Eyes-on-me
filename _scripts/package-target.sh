#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LEGACY_BUNDLE_DIR="${LEGACY_BUNDLE_DIR:-$(cd "$ROOT_DIR/.." && pwd)/dist/rust-monolith-bundle}"

TARGET_TRIPLE="${TARGET_TRIPLE:-$(rustc -vV | sed -n 's/^host: //p')}"
BUNDLE_NAME="${BUNDLE_NAME:-eyes-on-me-bundle-${TARGET_TRIPLE}}"
OUTPUT_DIR="${OUTPUT_DIR:-$ROOT_DIR/_dist/${BUNDLE_NAME}}"
PACKAGE_COPY_DB="${PACKAGE_COPY_DB:-0}"

SERVER_BIN_NAME="client-server"
AGENT_BIN_NAME="client-desktop"
SERVER_RUN_NAME="run-server.sh"
SERVER_PUBLIC_RUN_NAME="run-server-public.sh"
AGENT_RUN_NAME="run-agent.sh"

if [[ "$TARGET_TRIPLE" == *windows* ]]; then
  SERVER_BIN_NAME="${SERVER_BIN_NAME}.exe"
  AGENT_BIN_NAME="${AGENT_BIN_NAME}.exe"
  SERVER_RUN_NAME="run-server.bat"
  SERVER_PUBLIC_RUN_NAME="run-server-public.bat"
  AGENT_RUN_NAME="run-agent.bat"
fi

log() {
  printf '\n[%s] %s\n' "$1" "$2"
}

migrate_legacy_runtime_files() {
  local root_db="$ROOT_DIR/DB/eyes-on-me.db"
  local root_db_legacy="$ROOT_DIR/DB/amiokay.db"
  local legacy_db="$LEGACY_BUNDLE_DIR/data/amiokay.db"
  local root_config="$ROOT_DIR/client-desktop.config.json"
  local legacy_config="$LEGACY_BUNDLE_DIR/desktop-agent.config.json"

  mkdir -p "$ROOT_DIR/DB"

  if [ ! -f "$root_db" ] && [ -f "$root_db_legacy" ]; then
    log "RUN" "migrate legacy database name into rust-monolith/DB"
    cp "$root_db_legacy" "$root_db"
  fi

  if [ ! -f "$root_db" ] && [ -f "$legacy_db" ]; then
    log "RUN" "import legacy database into rust-monolith/DB"
    cp "$legacy_db" "$root_db"
  fi

  if [ ! -f "$root_config" ] && [ -f "$legacy_config" ]; then
    log "RUN" "import legacy agent config into rust-monolith root"
    cp "$legacy_config" "$root_config"
  fi
}

build_web() {
  if [ ! -d "$ROOT_DIR/web/node_modules" ]; then
    log "RUN" "pnpm install"
    (
      cd "$ROOT_DIR/web"
      pnpm install
    )
  fi

  log "RUN" "pnpm build"
  (
    cd "$ROOT_DIR/web"
    pnpm build
  )
}

build_server() {
  log "RUN" "cargo build --release -p client-server --target $TARGET_TRIPLE"
  cargo build \
    --release \
    --target "$TARGET_TRIPLE" \
    -p client-server \
    --manifest-path "$ROOT_DIR/Cargo.toml"
}

build_agent() {
  log "RUN" "cargo build --release -p client-desktop --target $TARGET_TRIPLE"
  cargo build \
    --release \
    --target "$TARGET_TRIPLE" \
    -p client-desktop \
    --manifest-path "$ROOT_DIR/Cargo.toml"
}

write_unix_script() {
  local path="$1"
  local content="$2"

  mkdir -p "$(dirname "$path")"
  printf '%s\n' "$content" > "$path"
  chmod +x "$path"
}

write_windows_script() {
  local path="$1"
  local content="$2"

  mkdir -p "$(dirname "$path")"
  printf '%s\r\n' "$content" > "$path"
}

write_runtime_scripts() {
if [[ "$TARGET_TRIPLE" == *windows* ]]; then
    write_windows_script "$OUTPUT_DIR/$SERVER_RUN_NAME" "@echo off
set ROOT_DIR=%~dp0
cd /d \"%ROOT_DIR%\"
if not exist \"%ROOT_DIR%DB\" mkdir \"%ROOT_DIR%DB\"
if \"%AMI_OKAY_HOST%\"==\"\" set AMI_OKAY_HOST=127.0.0.1
if \"%AMI_OKAY_PORT%\"==\"\" set AMI_OKAY_PORT=8787
if \"%AMI_OKAY_DATABASE_URL%\"==\"\" set AMI_OKAY_DATABASE_URL=sqlite://DB/eyes-on-me.db
\"%ROOT_DIR%bin\\$SERVER_BIN_NAME\""

    write_windows_script "$OUTPUT_DIR/$SERVER_PUBLIC_RUN_NAME" "@echo off
set ROOT_DIR=%~dp0
cd /d \"%ROOT_DIR%\"
if not exist \"%ROOT_DIR%DB\" mkdir \"%ROOT_DIR%DB\"
if \"%AMI_OKAY_HOST%\"==\"\" set AMI_OKAY_HOST=0.0.0.0
if \"%AMI_OKAY_PORT%\"==\"\" set AMI_OKAY_PORT=8787
if \"%AMI_OKAY_DATABASE_URL%\"==\"\" set AMI_OKAY_DATABASE_URL=sqlite://DB/eyes-on-me.db
\"%ROOT_DIR%bin\\$SERVER_BIN_NAME\""

    write_windows_script "$OUTPUT_DIR/$AGENT_RUN_NAME" "@echo off
set ROOT_DIR=%~dp0
cd /d \"%ROOT_DIR%\"
if \"%AGENT_SERVER_API_BASE_URL%\"==\"\" set AGENT_SERVER_API_BASE_URL=http://127.0.0.1:8787
\"%ROOT_DIR%bin\\$AGENT_BIN_NAME\""
    return
  fi

  write_unix_script "$OUTPUT_DIR/$SERVER_RUN_NAME" "#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR=\"\$(cd \"\$(dirname \"\$0\")\" && pwd)\"
cd \"\$ROOT_DIR\"
mkdir -p \"\$ROOT_DIR/DB\"
export AMI_OKAY_HOST=\"\${AMI_OKAY_HOST:-127.0.0.1}\"
export AMI_OKAY_PORT=\"\${AMI_OKAY_PORT:-8787}\"
export AMI_OKAY_DATABASE_URL=\"\${AMI_OKAY_DATABASE_URL:-sqlite://\$ROOT_DIR/DB/eyes-on-me.db}\"
exec \"\$ROOT_DIR/bin/$SERVER_BIN_NAME\""

  write_unix_script "$OUTPUT_DIR/$SERVER_PUBLIC_RUN_NAME" "#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR=\"\$(cd \"\$(dirname \"\$0\")\" && pwd)\"
cd \"\$ROOT_DIR\"
mkdir -p \"\$ROOT_DIR/DB\"
export AMI_OKAY_HOST=\"\${AMI_OKAY_HOST:-0.0.0.0}\"
export AMI_OKAY_PORT=\"\${AMI_OKAY_PORT:-8787}\"
export AMI_OKAY_DATABASE_URL=\"\${AMI_OKAY_DATABASE_URL:-sqlite://\$ROOT_DIR/DB/eyes-on-me.db}\"
exec \"\$ROOT_DIR/bin/$SERVER_BIN_NAME\""

  write_unix_script "$OUTPUT_DIR/$AGENT_RUN_NAME" "#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR=\"\$(cd \"\$(dirname \"\$0\")\" && pwd)\"
cd \"\$ROOT_DIR\"
export AGENT_SERVER_API_BASE_URL=\"\${AGENT_SERVER_API_BASE_URL:-http://127.0.0.1:8787}\"
export AGENT_CONFIG_PATH=\"\${AGENT_CONFIG_PATH:-\$ROOT_DIR/client-desktop.config.json}\"
export AGENT_NO_PROMPT=\"\${AGENT_NO_PROMPT:-1}\"
exec \"\$ROOT_DIR/bin/$AGENT_BIN_NAME\""
}

collect_bundle() {
  log "RUN" "collect build artifacts for $TARGET_TRIPLE"

  local preserved_bundle_db=""
  if [ "$PACKAGE_COPY_DB" != "1" ] && [ -f "$OUTPUT_DIR/DB/eyes-on-me.db" ]; then
    preserved_bundle_db="$(mktemp "${TMPDIR:-/tmp}/eyes-on-me-db.XXXXXX")"
    cp "$OUTPUT_DIR/DB/eyes-on-me.db" "$preserved_bundle_db"
  fi

  mkdir -p "$OUTPUT_DIR"
  find "$OUTPUT_DIR" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
  mkdir -p "$OUTPUT_DIR/bin" "$OUTPUT_DIR/DB"

  cp "$ROOT_DIR/target/$TARGET_TRIPLE/release/$SERVER_BIN_NAME" "$OUTPUT_DIR/bin/$SERVER_BIN_NAME"
  cp "$ROOT_DIR/target/$TARGET_TRIPLE/release/$AGENT_BIN_NAME" "$OUTPUT_DIR/bin/$AGENT_BIN_NAME"
  cp "$ROOT_DIR/README.md" "$OUTPUT_DIR/README.md"

  if [ "$PACKAGE_COPY_DB" = "1" ] && [ -f "$ROOT_DIR/DB/eyes-on-me.db" ]; then
    cp "$ROOT_DIR/DB/eyes-on-me.db" "$OUTPUT_DIR/DB/eyes-on-me.db"
  elif [ "$PACKAGE_COPY_DB" = "1" ] && [ -f "$ROOT_DIR/DB/amiokay.db" ]; then
    cp "$ROOT_DIR/DB/amiokay.db" "$OUTPUT_DIR/DB/eyes-on-me.db"
  elif [ -n "$preserved_bundle_db" ] && [ -f "$preserved_bundle_db" ]; then
    cp "$preserved_bundle_db" "$OUTPUT_DIR/DB/eyes-on-me.db"
  fi

  if [ -n "$preserved_bundle_db" ] && [ -f "$preserved_bundle_db" ]; then
    rm -f "$preserved_bundle_db"
  fi

  if [ -f "$ROOT_DIR/client-desktop.config.json" ]; then
    cp "$ROOT_DIR/client-desktop.config.json" "$OUTPUT_DIR/client-desktop.config.json"
  fi

  write_runtime_scripts

  cat > "$OUTPUT_DIR/client-desktop.config.example.json" <<'EOF'
{
  "server_api_base_url": "http://127.0.0.1:8787",
  "device_id": "my-device",
  "agent_name": "client-desktop",
  "api_token": "dev-agent-token"
}
EOF

  find "$OUTPUT_DIR" -name '.DS_Store' -exec rm -f {} +
  find "$OUTPUT_DIR" -name '__MACOSX' -prune -exec rm -rf {} +

  log "OK" "Eyes on Me bundle ready at $OUTPUT_DIR"
}

main() {
  migrate_legacy_runtime_files
  build_web
  build_server
  build_agent
  collect_bundle
}

main "$@"
