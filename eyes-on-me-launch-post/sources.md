# Sources

本次文章只使用仓库内材料，没有外部网络来源。

1. `README.md`
   - Type: local project documentation
   - Why it matters: 中文主说明文档，确认项目定位、页面结构、运行方式、技术栈
   - Time relevance: current repo state
   - Reliability note: 高，属于项目内一手说明

2. `README_EN.md`
   - Type: local project documentation
   - Why it matters: 英文说明文档，用于交叉确认页面结构和功能表述
   - Time relevance: current repo state
   - Reliability note: 高

3. `web/src/views/HomeView.vue`
   - Type: frontend source file
   - Why it matters: 确认首页当前已经承担全局分析页角色，并包含设备卡片、全局高频窗口、浏览器域名累计
   - Time relevance: current repo state
   - Reliability note: 高

4. `web/src/views/DeviceDetailView.vue`
   - Type: frontend source file
   - Why it matters: 确认单设备明细页当前职责是查看最近活动切换
   - Time relevance: current repo state
   - Reliability note: 高

5. `web/src/views/DeviceAnalysisView.vue`
   - Type: frontend source file
   - Why it matters: 确认单设备分析页包含应用 treemap、应用累计、域名累计和时间范围切换
   - Time relevance: current repo state
   - Reliability note: 高

6. `web/src/lib/device-treemap.ts`
   - Type: frontend source file
   - Why it matters: 确认 treemap 当前只基于 `appUsage`，不混入 `domainUsage`
   - Time relevance: current repo state
   - Reliability note: 高

7. `client-server/src/app_state.rs`
   - Type: backend source file
   - Why it matters: 确认后端存在 `app_usage` 和 `domain_usage` 两套聚合逻辑
   - Time relevance: current repo state
   - Reliability note: 高

8. `client-desktop/src/browser.rs`
   - Type: desktop collector source file
   - Why it matters: 确认桌面端会尝试识别浏览器 page title / URL / domain
   - Time relevance: current repo state
   - Reliability note: 高

9. `image/Home.png`
   - Type: local screenshot asset
   - Why it matters: 首页 / 全局分析截图
   - Time relevance: current repo state
   - Reliability note: 高

10. `image/Detail.png`
    - Type: local screenshot asset
    - Why it matters: 单设备明细截图
    - Time relevance: current repo state
    - Reliability note: 高

11. `image/Analyze.png`
    - Type: local screenshot asset
    - Why it matters: 单设备分析截图
    - Time relevance: current repo state
    - Reliability note: 高
