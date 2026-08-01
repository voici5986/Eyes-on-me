# Eyes on Me

[English](README_EN.md) | [中文](README.md)

## 1. What is this project for?

If you ever feel like this:

- You just open your browser to check something
- Then look up and 3 hours are gone
- You thought you were working the whole time
- But actually, you've switched between apps, windows, web pages, and domains dozens of times

Then `Eyes on Me` is here to expose that.

It does three things:

- Collects current foreground app, window title, and browser context on the desktop
- Continuously logs into the database on the server, forming device-level activity details
- Displays "what have I been doing this time" directly in the web interface

Now you can view these pages:

- `/` - Home / global analysis page, showing device cards, top windows, and browser domain usage
- `/devices/:deviceId` - Historical timeline with date/range, app/domain/category filters, pagination, screenshots, sessions, and granular deletion
- `/devices/:deviceId/analysis` - Single device analysis page, view usage profile of a machine
- `/reports` - Deterministic or AI-polished reports, block preferences, history, range export, and auto-export
- `/assistant` - Record-grounded work assistant with conversation history, natural-language ranges, streamed replies, template and optional AI modes
- `/memory` - Search activity, screenshot OCR, and report memories
- `/media` - Agent permissions/privacy/spool diagnostics, media capacity, OCR retry, deletion, and cleanup
- `/settings` - Historical category rules, multi-segment working hours, report preferences, and settings backup/restore

The analysis page supports these time ranges:

- `3h`
- `6h`
- `today`
- `1d`
- `1w`
- `1m`
- `all`

In one sentence:

**This is not just a "monitoring demo". This is a Rust monolithic project that turns your computer usage trajectory into something you can view, replay, and analyze.**

Recording can be paused or resumed per device from the Dashboard. The Agent checks the lightweight control endpoint every five seconds and persists the last applied state; foreground and tab changes remain native event-driven and do not start AppleScript or high-frequency helper processes.

## 2. Screenshots

All screenshots are stored in [`image/`](image/):

### Home / Global Analysis

![Home](image/Home.png)

### Device Detail

![Detail](image/Detail.png)

### Device Analysis

![Analyze](image/Analyze.png)

## 3. How to operate

### Usage

Download the release directly. On first run, the desktop collector will generate a JSON config file by default.

All commands below are executed in this directory:

```bash
cd /Users/wong/Code/RustLang/Eyes_on_me
```

### Start the server

```bash
# Local
./_scripts/run-server.sh

# For LAN / public access
./_scripts/run-server-public.sh
```

Default address:

- Default listen address: `127.0.0.1:8787` (local access only)
- Local access URL: `http://127.0.0.1:8787`
- Default database file: `DB/eyes-on-me.db`
- The server binary embeds the web UI by default, so it does not require an external `web/dist`

The server stores sortable timestamps as fixed-millisecond UTC values, while `today`, daily summaries, and work schedules still use the server's local timezone. On the first upgraded startup, legacy RFC3339 offset variants are normalized transactionally and recorded in `eyes_on_me_schema_migrations`. Instants do not change, but backing up `DB/eyes-on-me.db` before an important upgrade is still recommended.

Agent write endpoints require a Bearer Token. The development default is `dev-agent-token`; use the same long random value on both sides for LAN or public deployments:

```bash
EYES_ON_ME_AGENT_API_TOKEN='replace-with-a-long-random-agent-token' \
EYES_ON_ME_DASHBOARD_TOKEN='replace-with-a-long-random-dashboard-token' \
./_scripts/run-server-public.sh

AGENT_API_TOKEN='replace-with-a-long-random-agent-token' \
AGENT_SERVER_API_BASE_URL='http://server-address:8787' \
./_scripts/run-agent.sh
```

`run-server-public.sh` listens on `0.0.0.0` and requires both tokens to contain at least 24 characters. Dashboard login uses an `HttpOnly`, `SameSite=Strict` cookie. Set `EYES_ON_ME_SECURE_COOKIE=1` behind HTTPS; a VPN or HTTPS reverse proxy is still recommended.

### Start the desktop collector

```bash
./_scripts/run-agent.sh
```

Screenshots are disabled by default. When enabled, they are captured only for active foreground changes; idle, locked, `anonymize`, and `skip` activity never produces screenshots. The default output is an active-window JPEG capped at 1920 pixels wide:

```bash
EYES_ON_ME_SCREENSHOTS=1 EYES_ON_ME_SCREENSHOT_COOLDOWN_SECS=30 ./_scripts/run-agent.sh
```

The Agent atomically writes each event and its bound screenshot to `client-desktop.spool` before upload. Failed deliveries survive restarts and are removed only after both uploads succeed. The default spool cap is 512MB.

The server runs bounded-concurrency OCR with `tesseract`, stores originals under `DB/media`, and generates independent JPEG thumbnails. Similar images reuse OCR. The default retention is 7 days with a 2GB media cap, configurable with `EYES_ON_ME_MEDIA_RETENTION_DAYS` and `EYES_ON_ME_MEDIA_TOTAL_MAX_BYTES`. Set `EYES_ON_ME_OCR_COMMAND=off` to disable OCR, `EYES_ON_ME_OCR_LANGUAGE=eng+chi_sim` for installed language packs, or `EYES_ON_ME_OCR_REDACT_TERMS=token,password` to redact matching OCR lines before storage/indexing.

AI report polishing and embeddings are optional. Without them, deterministic reports and SQLite full-text memory search still work:

```bash
EYES_ON_ME_AI_BASE_URL='https://api.openai.com/v1' \
EYES_ON_ME_AI_API_KEY='your-api-key' \
EYES_ON_ME_AI_MODEL='your-chat-model' \
EYES_ON_ME_EMBEDDING_MODEL='your-embedding-model' \
./_scripts/run-server.sh
```

Optional screenshot mirroring supports WebDAV and S3/MinIO. Local storage remains authoritative, while remote failures are visible and retryable:

```bash
EYES_ON_ME_REMOTE_PROVIDER=webdav \
EYES_ON_ME_WEBDAV_URL='https://dav.example.com/archive' \
EYES_ON_ME_WEBDAV_USERNAME='user' \
EYES_ON_ME_WEBDAV_PASSWORD='password' \
./_scripts/run-server.sh

EYES_ON_ME_REMOTE_PROVIDER=s3 \
EYES_ON_ME_S3_ENDPOINT='https://s3.example.com' \
EYES_ON_ME_S3_BUCKET='activity-archive' \
EYES_ON_ME_S3_REGION='us-east-1' \
EYES_ON_ME_S3_ACCESS_KEY='access-key' \
EYES_ON_ME_S3_SECRET_KEY='secret-key' \
./_scripts/run-server.sh
```

Set `EYES_ON_ME_INTEGRATION_TOKEN` to enable the authenticated JSON-RPC MCP endpoint at `POST /mcp`. It exposes current context, timeline, work sessions, memory search, reports, and media status without giving integrations direct SQLite access.

To temporarily change the server address:

```bash
AGENT_SERVER_API_BASE_URL=http://127.0.0.1:8787 ./_scripts/run-agent.sh
```

### Open the page

```text
http://127.0.0.1:8787/
```

You can switch directly on the home page:

- Last 3 hours
- Last 6 hours
- Today
- Last 1 day
- Last 1 week
- Last 1 month
- All history

### Local frontend development

```bash
./_scripts/run-web-dev.sh
```

Frontend development URL:

- `http://127.0.0.1:5173`

Vite already proxies `/api` and `/health` to the local server at `http://127.0.0.1:8787`.

If you want to force the server to read a specific external static directory, you can still override it:

```bash
EYES_ON_ME_WEB_DIST=/absolute/path/to/web/dist ./_scripts/run-server.sh
```

### Local development mode

You no longer need to package the project every time before testing.

Open 3 terminals:

```bash
# Terminal 1: server
./_scripts/run-server.sh

# Terminal 2: desktop collector
./_scripts/run-agent.sh

# Terminal 3: frontend dev server
./_scripts/run-web-dev.sh
```

Then open:

- `http://127.0.0.1:5173`

If you only want the startup summary:

```bash
./_scripts/run-dev.sh
```

### One-click packaging

```bash
./_scripts/package.sh
```

### Full acceptance

Build once, then run the isolated acceptance suite. It uses a temporary SQLite database, media directory, and `127.0.0.1:18787`, then cleans up without touching production data:

```bash
cargo build --workspace
./_scripts/test-acceptance.sh
```

Default output to:

- `_dist/eyes-on-me-bundle-<host-target>`

Current packaging behavior:

- `client-server` embeds `web/dist` directly during build
- The bundle no longer copies a separate `web/dist` directory by default
- Preserves the existing `DB/eyes-on-me.db` inside the bundle by default
- No longer force-copies the root `DB/eyes-on-me.db` into the bundle by default
- If you explicitly want to package the root database into the bundle:

```bash
PACKAGE_COPY_DB=1 ./_scripts/package.sh
```

To specify platform:

```bash
TARGET_TRIPLE=x86_64-unknown-linux-gnu ./_scripts/package-target.sh
```

## Current Linux collection notes

> Using Linux, what interface do you need (dog)

Linux is no longer a stub, it already has the first version of foreground window collection.

Current conditions:

- Requires graphical desktop environment
- Requires `xprop`
- More suitable for X11 / XWayland

Current capabilities:

- Identify foreground app
- Identify window title
- In browser scenarios, try to infer domain from page title
- Report to server and aggregate into the home page / per-device analysis page

Current limitations:

- Browser domain recognition is not as complete as macOS
- Pure Wayland native window scenarios need further compatibility improvement
- When upgrading to new version for the first time, if the directory only has old `amiokay.db`, the server will automatically migrate to new `eyes-on-me.db`

## 4. Technical implementation

### Server

The server is a Rust process responsible for:

- Hosting Vue static pages
- Receiving `client-desktop` reports
- Writing to SQLite
- Providing summary/detail/analysis APIs
- Providing arbitrary-range timelines, sessions, follow-up clues, pagination, and transactional bulk deletion
- Validating originals, generating thumbnails, bounded/similarity-aware OCR, retention/capacity cleanup, and granular deletion
- Generating editable reports and optional embedding-backed memories
- Managing historical category backfill, multi-segment schedules, report blocks, assistant history, and settings backups
- Optionally mirroring media to WebDAV/S3 and serving an independently authenticated MCP boundary
- Protecting dashboard APIs, media, SSE, reports, and memory with cookie auth
- Pushing latest snapshots to browser via SSE

Main technologies:

- `Rust`
- `axum`
- `tokio`
- `sqlx`
- `SQLite`
- `tower-http`
- `SSE`

Main APIs:

- `GET /health`
- `GET /api/current`
- `GET /api/devices`
- `GET /api/devices/:deviceId`
- `GET /api/timeline?date=...&deviceId=...&app=...&domain=...&category=...`
- `GET /api/sessions?date=...&deviceId=...`
- `POST /api/activities/bulk-delete`
- `GET /api/analysis?range=...`
- `GET /api/devices/:deviceId/analysis?range=...`
- `PUT /api/devices/:deviceId/recording`
- `GET /api/stream`
- `POST /api/auth/login`
- `POST /api/agent/screenshots/:eventId`
- `GET /api/devices/:deviceId/screenshots`
- `GET /api/screenshots/:screenshotId/thumbnail`
- `DELETE /api/screenshots/:screenshotId`
- `POST /api/screenshots/:screenshotId/ocr`
- `DELETE /api/activities/:eventId`
- `GET /api/media/status`
- `POST /api/media/cleanup`
- `GET /api/export/activities?format=csv|json`
- `GET|PUT /api/reports/:date`
- `POST /api/reports/:date/generate`
- `GET /api/reports?start=...&end=...`
- `GET /api/reports/export?start=...&end=...`
- `GET /api/memory`
- `POST /api/memory/reindex`
- `GET|PUT /api/settings/review`
- `GET /api/assistant/conversations`
- `POST /api/assistant/stream`
- `GET /api/media/remote`
- `POST /mcp`
- `POST /api/agent/activity`
- `POST /api/agent/status`

### Frontend

The frontend is a lightweight Vue workbench, not a fancy admin panel, just for "viewing data".

Main technologies:

- `Vite`
- `Vue 3`
- `TypeScript`
- `vue-router`

Current frontend capabilities:

- Home / global analysis
- Single device details
- Single device analysis
- Time range switching
- SSE auto-refresh
- Historical timeline/viewer, OCR search, combined filters, sessions, and granular deletion
- Device recording controls, report history/preferences, assistant history/streaming, and memory search
- Category/schedule settings, backup/restore, and remote mirror diagnostics

### Desktop collector

`client-desktop` is also written in Rust.

Platform implementations:

- macOS: native `NSWorkspace` + `AXObserver` + Accessibility/CoreGraphics fallback, with no runtime AppleScript
- Windows: real-time switching + periodic sampling
- Linux: `xprop` polling

Collection process:

1. Read current foreground app and window info
2. In browser scenarios, supplement page title / URL / domain as much as possible
3. Detect idle / locked state separately so active time is cut off correctly
4. Apply `record`, `anonymize`, or `skip` privacy rules locally
5. Persist the event and optional screenshot to the disk spool, then upload in order
6. Server writes to DB, webpage updates automatically

Current collection mode:

- Foreground switches are reported immediately
- Long stays get sampled every 15 seconds
- Analysis caps long gaps so sparse historical data does not accidentally become all-day activity

### Why SSE instead of WebSocket

The current chain is actually simple:

- `client-desktop -> client-server` uses HTTP POST
- `client-server -> browser` uses SSE

The reasons are simple:

- The page is mainly for viewing data, not bidirectional real-time collaboration
- Browser only needs to continuously receive pushes
- SSE is light enough and easier to maintain

If we need to do control commands, remote operations, bidirectional communication in the future, we can add WebSocket then.

## 社区

[LINUX DO](https://linux.do/)

## License

This project no longer uses GPL.

It is now released under the custom license in the repository root:

- Only natural persons may use, copy, modify, and distribute it for personal, non-commercial purposes
- If you modify it, distribute it, or deploy it for others to use over a network, you must provide the corresponding source code
- Derivative works must remain under the same license
- Any commercial use, internal company use, client work, paid service, SaaS, deployment, or support requires separate written commercial authorization

This means the project is `source-available`, not an OSI-approved open source project.

See [LICENSE](/Users/wong/Code/RustLang/Eyes_on_me/LICENSE) for the full terms.
