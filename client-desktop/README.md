# client-desktop

`client-desktop` 是 Eyes on Me 的桌面采集端，负责监听当前前台应用变化并把活动上报到 Rust 服务端。

## 运行

在 `rust-monolith` 根目录执行：

```bash
./_scripts/run-agent.sh
```

也可以直接用 Cargo：

```bash
cargo run -p client-desktop
```

## 配置

启动时会读取或写入根目录的 `client-desktop.config.json`。

配置写入采用临时文件、`fsync` 和原子重命名；有效旧配置会保存为 `client-desktop.config.json.bak`。主配置损坏时优先从备份恢复；主文件和备份都损坏时进入 fail-safe 默认值（本地服务端、截图关闭、采集规则为 `skip`），并保留损坏文件供排查。

可配置项：

- `server_api_base_url`
- `device_id`
- `agent_name`
- `api_token`
- `capture_filters.default_mode`: `record` / `anonymize` / `skip`
- `capture_filters.app_rules[]` / `capture_filters.domain_rules[]`: `{ "pattern": "...", "mode": "..." }`
- `screenshots.enabled` / `screenshots.cooldown_secs`
- `screenshots.format`: `jpeg` / `png`
- `screenshots.max_width` / `screenshots.jpeg_quality`
- `screenshots.display`: `active` / `primary` / `all`
- `spool.max_bytes`: 磁盘待发送队列上限，默认 512MB

支持的环境变量：

- `AGENT_SERVER_API_BASE_URL`
- `AGENT_API_TOKEN`
- `AGENT_DEVICE_ID`
- `AGENT_NAME`
- `EYES_ON_ME_SCREENSHOTS`
- `EYES_ON_ME_SCREENSHOT_COOLDOWN_SECS`

## 平台实现

- macOS: `NSWorkspace` 应用激活通知 + Accessibility 窗口/Tab 事件；Accessibility 原生读取浏览器 URL，CoreGraphics 作为标题兜底。运行时不调用 AppleScript。浏览器和 Terminal 每 5 秒做一次原生补扫，普通应用只随 15 秒心跳补扫
- Windows: `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` + 轮询兜底
- Linux: `xprop` 轮询当前激活窗口，采集应用、窗口标题和 PID

macOS 要完整识别 Terminal/浏览器 Tab 和浏览器 URL，需要给启动 Agent 的终端或 Agent 本体开启“系统设置 -> 隐私与安全性 -> 辅助功能”。权限不足时应用切换仍会立即上报，窗口标题降级为 CoreGraphics，Tab 变化可能延迟到补扫且 URL 可能为空。

截图默认关闭。开启后只对活跃的 `foreground_changed` 事件按冷却时间截图；`anonymize` / `skip`、空闲和锁屏事件不会截图。macOS `active` 使用 CoreGraphics 窗口 ID，`primary` 只截主显示器，`all` 截全部显示器。截图本地缩放并编码后，与事件一起原子写入 `client-desktop.spool`；网络失败或进程重启后继续发送，活动和截图都成功才删除条目。

隐私模式在采集端执行：

- `record`：保存应用、窗口/Tab、浏览器 URL/域名，并按截图策略处理。
- `anonymize`：保留应用名和时间，移除 PID、窗口/Tab、URL、域名并禁止截图。
- `skip`：不产生事件，也不截图。

旧版 `ignored_apps` / `ignored_domains` 会在读取后自动迁移为 `anonymize` 规则。Agent 每 60 秒上报辅助功能、屏幕录制、截图策略、隐私规则数量和 spool 积压；Dashboard 的 `/media` 页面可查看。

Dashboard 可按设备暂停或恢复采集。Agent 每 5 秒请求一次轻量控制端点，只更新内存原子状态，不调用窗口 API 或外部命令；暂停时 watcher 继续处理系统 RunLoop，但不读取、截图或上报活动。最后一次状态和 revision 保存在 `client-desktop.control.json`，网络中断或重启不会绕过暂停状态，恢复后会立即上报当前前台上下文。

Linux 当前注意点：

- 需要图形桌面会话
- 需要 `xprop` 在 `PATH` 里
- 目前更适合 X11 / XWayland 环境
- 浏览器场景会尝试从窗口标题里反推域名
- 浏览器域名提取能力不如 macOS，优先依赖窗口标题

## 测试

```bash
cargo test
```
