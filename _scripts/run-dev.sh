#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cat <<EOF
开发态请分别开 3 个终端：

1. 服务端
   $ROOT_DIR/_scripts/run-server.sh

2. 桌面采集端
   $ROOT_DIR/_scripts/run-agent.sh

3. 前端 Vite
   $ROOT_DIR/_scripts/run-web-dev.sh

打开：
  http://127.0.0.1:5173
EOF
