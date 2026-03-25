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
- `/devices/:deviceId` - Single device detail page, view recent activity switches
- `/devices/:deviceId/analysis` - Single device analysis page, view usage profile of a machine

The analysis page supports these time ranges:

- `3h`
- `6h`
- `1d`
- `1w`
- `1m`
- `all`

In one sentence:

**This is not just a "monitoring demo". This is a Rust monolithic project that turns your computer usage trajectory into something you can view, replay, and analyze.**

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

- `http://127.0.0.1:8787`
- Default database file: `DB/eyes-on-me.db`

### Start the desktop collector

```bash
./_scripts/run-agent.sh
```

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
- Last 1 day
- Last 1 week
- Last 1 month
- All history

### Local frontend development

```bash
cd web
pnpm install
pnpm dev
```

### One-click packaging

```bash
./_scripts/package.sh
```

Default output to:

- `_dist/eyes-on-me-bundle-<host-target>`

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
- `GET /api/analysis?range=...`
- `GET /api/devices/:deviceId/analysis?range=...`
- `GET /api/stream`
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

### Desktop collector

`client-desktop` is also written in Rust.

Platform implementations:

- macOS: `NSWorkspace`
- Windows: `SetWinEventHook`
- Linux: `xprop` polling

Collection process:

1. Read current foreground app and window info
2. In browser scenarios, supplement page title / URL / domain as much as possible
3. Send via HTTP POST to server
4. Server writes to DB, webpage updates automatically

### Why SSE instead of WebSocket

The current chain is actually simple:

- `client-desktop -> client-server` uses HTTP POST
- `client-server -> browser` uses SSE

The reasons are simple:

- The page is mainly for viewing data, not bidirectional real-time collaboration
- Browser only needs to continuously receive pushes
- SSE is light enough and easier to maintain

If we need to do control commands, remote operations, bidirectional communication in the future, we can add WebSocket then.
