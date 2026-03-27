<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import { fetchAnalysisOverview, fetchDevices, peekAnalysisOverview, peekDevices } from "../api";
import {
  activityDurationMs,
  activityHeadline,
  activitySubline,
  activityUrl,
  formatDateTime,
  formatDuration,
  formatDurationLong,
  formatTime,
  isFreshActivity,
  usageShare
} from "../lib/activity";
import { ANALYSIS_RANGE_OPTIONS, analysisRangeLabel, normalizeAnalysisRange } from "../lib/analysis-range";
import DeviceSummaryCard from "../components/DeviceSummaryCard.vue";
import type { AnalysisOverviewResponse, AnalysisRange, DevicesResponse } from "../types";

const props = defineProps<{
  connection: "connecting" | "live" | "closed";
  nowMs: number;
  refreshToken: number;
}>();

const route = useRoute();
const router = useRouter();
const selectedRange = computed(() => normalizeAnalysisRange(route.query.range));

const initialAnalysis = peekAnalysisOverview(selectedRange.value);
const initialDevices = peekDevices();
const loading = ref(!initialAnalysis || !initialDevices);
const error = ref<string | null>(null);
const analysisResponse = ref<AnalysisOverviewResponse | null>(initialAnalysis);
const devicesResponse = ref<DevicesResponse | null>(initialDevices);

const topAppUsage = computed(() => analysisResponse.value?.topAppUsage ?? []);
const topDomainUsage = computed(() => analysisResponse.value?.topDomainUsage ?? []);
const topBrowserUsage = computed(() => analysisResponse.value?.topBrowserUsage ?? []);
const topDomainPagesByKey = computed(() => {
  const map = new Map<string, AnalysisOverviewResponse["topBrowserUsage"][number]["domains"][number]["pages"]>();

  for (const browser of topBrowserUsage.value) {
    for (const domain of browser.domains) {
      if (!map.has(domain.key)) {
        map.set(domain.key, domain.pages);
      }
    }
  }

  return map;
});
const devices = computed(() => analysisResponse.value?.devices ?? []);
const deviceOverviewById = computed(() => {
  return new Map((devicesResponse.value?.devices ?? []).map((item) => [item.device.deviceId, item]));
});
const deviceCards = computed(() =>
  devices.value.map((device) => {
    const overview = deviceOverviewById.value.get(device.deviceId);
    const currentDevice = overview?.device;
    const latestStatus = overview?.latestStatus;

    return {
      deviceId: device.deviceId,
      headline: currentDevice ? activityHeadline(currentDevice) : (device.latestStatusText || device.currentLabel),
      metaLine: currentDevice
        ? `${currentDevice.app.name} · ${currentDevice.platform} · ${
          isFreshActivity(currentDevice, props.nowMs)
            ? `最近上报 ${formatDuration(activityDurationMs(currentDevice, props.nowMs))} 前`
            : "当前状态已过期"
        }`
        : `${device.platform} · 当前设备在线`,
      summaryLine: currentDevice
        ? (latestStatus?.statusText || activitySubline(currentDevice))
        : (device.latestStatusText || device.currentLabel),
      url: currentDevice ? activityUrl(currentDevice) : null,
      topBadge: `${device.eventCount} 次切换`,
      footerMeta: [
        `使用时长 ${formatDurationLong(device.totalTrackedMs)}`,
        `最近更新 ${formatTime(currentDevice?.ts || device.lastSeen)}`
      ]
    };
  })
);
const hasAnyAnalysis = computed(() =>
  (analysisResponse.value?.deviceCount ?? 0) > 0 ||
  topAppUsage.value.length > 0 ||
  topDomainUsage.value.length > 0 ||
  topBrowserUsage.value.length > 0
);

async function loadData(force = false) {
  loading.value = !analysisResponse.value || !devicesResponse.value;

  try {
    const [analysis, devices] = await Promise.all([
      fetchAnalysisOverview(selectedRange.value, force),
      fetchDevices(force)
    ]);
    analysisResponse.value = analysis;
    devicesResponse.value = devices;
    error.value = null;
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
  }
}

async function updateRange(range: AnalysisRange) {
  if (range === selectedRange.value) {
    return;
  }

  await router.replace({
    query: {
      ...route.query,
      range
    }
  });
}

onMounted(() => {
  if (loading.value) {
    void loadData();
  }
});
watch(selectedRange, () => {
  analysisResponse.value = peekAnalysisOverview(selectedRange.value);
  devicesResponse.value = peekDevices();
  void loadData();
});
watch(() => props.refreshToken, () => {
  void loadData(true);
});
</script>

<template>
  <section v-if="loading" class="panel">
    <p>Loading analysis overview...</p>
  </section>

  <section v-else-if="error" class="panel error-panel">
    <p>{{ error }}</p>
  </section>

  <template v-else-if="analysisResponse">
    <section class="page-actions">
      <!-- <RouterLink class="button-link" to="/">返回首页</RouterLink> -->
      <span class="muted">连接状态：{{ connection }}</span>
      <span class="muted">统计范围：{{ analysisRangeLabel(selectedRange) }}</span>
      <span class="muted">生成时间：{{ formatTime(analysisResponse.generatedAt) }}</span>
    </section>

    <section class="panel range-panel">
      <div class="panel-header">
        <h2>时间范围</h2>
        <span>{{ analysisRangeLabel(selectedRange) }}</span>
      </div>
      <div class="range-switcher">
        <button
          v-for="option in ANALYSIS_RANGE_OPTIONS"
          :key="option.value"
          type="button"
          class="range-chip"
          :class="{ active: option.value === selectedRange }"
          @click="updateRange(option.value)"
        >
          {{ option.label }}
        </button>
      </div>
    </section>

    <section v-if="!hasAnyAnalysis" class="panel empty-state">
      <span class="eyebrow">No Activity Yet</span>
      <h2 class="analysis-title">当前还没有可分析的活动记录</h2>
      <p class="analysis-lede">
        这通常不是页面问题，而是当前这个时间范围内还没有数据，或者当前这套 Eyes on Me 服务还没有积累到活动记录。
        先启动当前目录下的 server 和 client-desktop，等前台应用上报几次后，这里就会出现设备、窗口和域名时长统计。
      </p>
      <div class="placeholder-stack">
        <div class="placeholder-card">
          <strong>先启动服务端</strong>
          <p><code>/Users/wong/Code/RustLang/Eyes_on_me/_scripts/run-server.sh</code></p>
        </div>
        <div class="placeholder-card">
          <strong>再启动客户端</strong>
          <p><code>/Users/wong/Code/RustLang/Eyes_on_me/_scripts/run-agent.sh</code></p>
        </div>
        <div class="placeholder-card">
          <strong>确认不是旧 bundle</strong>
          <p>如果你之前跑的是外层旧 bundle，或者还在看老数据库，那边的数据不会自动出现在这里。</p>
        </div>
      </div>
    </section>

    <template v-else>
    <section class="analysis-summary">
      <article class="panel">
        <span class="eyebrow">Analysis Ledger</span>
        <h2 class="analysis-title">全局累计使用画像</h2>
        <p class="analysis-lede">基于 {{ analysisRangeLabel(selectedRange) }} 的活动记录，按设备、窗口和域名重新聚合使用时长。</p>

        <div class="stats-row">
          <div class="metric-block">
            <span class="label">活动总时长</span>
            <strong>{{ formatDurationLong(analysisResponse.totalTrackedMs) }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">工作时段活跃</span>
            <strong>{{ formatDurationLong(analysisResponse.workTrackedMs) }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">浏览器时长</span>
            <strong>{{ formatDurationLong(analysisResponse.browserTrackedMs) }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">应用数</span>
            <strong>{{ analysisResponse.appCount }}</strong>
          </div>
        </div>

        <div class="stats-row secondary">
          <div class="metric-block">
            <span class="label">累计记录时长</span>
            <strong>{{ formatDurationLong(analysisResponse.totalTrackedMs) }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">设备数量</span>
            <strong>{{ analysisResponse.deviceCount }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">最近生成</span>
            <strong>{{ formatDateTime(analysisResponse.generatedAt) }}</strong>
          </div>
        </div>
      </article>
    </section>

    <section class="grid">
      <article class="panel">
        <div class="panel-header">
          <h2>设备累计时长</h2>
          <span>{{ devices.length }}</span>
        </div>

        <ul class="card-list">
          <DeviceSummaryCard
            v-for="device in deviceCards"
            :key="device.deviceId"
            :title="device.deviceId"
            :headline="device.headline"
            :meta-line="device.metaLine"
            :summary-line="device.summaryLine"
            :url="device.url"
            :top-badge="device.topBadge"
            :footer-meta="device.footerMeta"
            :actions="[
              { label: '查看明细', to: `/devices/${encodeURIComponent(device.deviceId)}` },
              { label: '分析页', to: `/devices/${encodeURIComponent(device.deviceId)}/analysis?range=${selectedRange}` }
            ]"
          />
        </ul>
      </article>

      <article class="panel">
        <div class="panel-header">
          <h2>全局高频窗口</h2>
          <span>{{ topAppUsage.length }}</span>
        </div>

        <ul class="usage-list">
          <li v-for="bucket in topAppUsage" :key="bucket.key" class="usage-item">
            <div class="usage-copy">
              <strong>{{ bucket.label }}</strong>
              <p>{{ bucket.sublabel || "未提供附加信息" }}</p>
              <div class="usage-bar">
                <span :style="{ width: `${usageShare(analysisResponse.totalTrackedMs, bucket.totalTrackedMs)}%` }" />
              </div>
              <span class="inline-meta">最近 {{ formatDateTime(bucket.lastSeen) }}</span>
            </div>
            <div class="usage-side">
              <strong>{{ formatDurationLong(bucket.totalTrackedMs) }}</strong>
              <span class="inline-meta">{{ bucket.sessions }} 次进入</span>
            </div>
          </li>
        </ul>
      </article>
    </section>

    <section class="panel">
      <div class="panel-header">
        <h2>浏览器域名累计</h2>
        <span>{{ topDomainUsage.length }}</span>
      </div>

      <ul class="usage-list">
        <li v-for="bucket in topDomainUsage" :key="bucket.key" class="usage-item domain-usage-item">
          <div class="usage-copy">
            <details class="browser-tree domain-tree" :open="bucket.totalTrackedMs === topDomainUsage[0]?.totalTrackedMs">
              <summary class="browser-tree-summary domain-tree-summary">
                <div class="domain-tree-body">
                  <div class="domain-tree-main">
                    <div class="domain-tree-heading">
                      <strong>{{ bucket.label }}</strong>
                      <p>{{ bucket.sublabel || "未提供页面标题" }}</p>
                    </div>
                    <div class="domain-tree-stats">
                      <strong>{{ formatDurationLong(bucket.totalTrackedMs) }}</strong>
                      <span class="inline-meta">{{ usageShare(analysisResponse.totalTrackedMs, bucket.totalTrackedMs).toFixed(1) }}%</span>
                    </div>
                  </div>
                  <div class="domain-tree-meta">
                    <span class="inline-meta">页面 {{ (topDomainPagesByKey.get(bucket.key) ?? []).length }}</span>
                    <span class="inline-meta">访问 {{ bucket.sessions }} 次</span>
                    <span class="inline-meta">最近 {{ formatDateTime(bucket.lastSeen) }}</span>
                  </div>
                </div>
              </summary>
              <div class="browser-tree-pages">
                <div
                  v-for="page in topDomainPagesByKey.get(bucket.key) ?? []"
                  :key="page.key"
                  class="browser-tree-page"
                >
                  <div class="browser-tree-copy">
                    <strong>{{ page.label }}</strong>
                    <code v-if="page.url" class="url">{{ page.url }}</code>
                  </div>
                  <div class="browser-tree-side">
                    <strong>{{ formatDuration(page.totalTrackedMs) }}</strong>
                  </div>
                </div>
              </div>
            </details>
            <div class="usage-bar">
              <span :style="{ width: `${usageShare(analysisResponse.totalTrackedMs, bucket.totalTrackedMs)}%` }" />
            </div>
          </div>
        </li>
      </ul>
    </section>

    <section class="panel">
      <div class="panel-header">
        <h2>浏览器站点层级</h2>
        <span>{{ topBrowserUsage.length }}</span>
      </div>

      <ul class="usage-list">
        <li v-for="browser in topBrowserUsage" :key="browser.key" class="usage-item">
          <div class="usage-copy">
            <strong>{{ browser.label }}</strong>
            <p>{{ browser.family }} · {{ browser.domains.length }} 个域名</p>
            <div class="usage-bar">
              <span :style="{ width: `${usageShare(analysisResponse.totalTrackedMs, browser.totalTrackedMs)}%` }" />
            </div>
            <span class="inline-meta">最近 {{ formatDateTime(browser.lastSeen) }}</span>
            <details
              v-for="domain in browser.domains.slice(0, 3)"
              :key="domain.key"
              class="browser-tree"
            >
              <summary class="browser-tree-summary">
                <span>{{ domain.label }}</span>
                <span>{{ formatDurationLong(domain.totalTrackedMs) }}</span>
              </summary>
              <div class="browser-tree-pages">
                <div
                  v-for="page in domain.pages.slice(0, 4)"
                  :key="page.key"
                  class="browser-tree-page"
                >
                  <div class="browser-tree-copy">
                    <strong>{{ page.label }}</strong>
                    <code v-if="page.url" class="url">{{ page.url }}</code>
                  </div>
                  <div class="browser-tree-side">
                    <strong>{{ formatDuration(page.totalTrackedMs) }}</strong>
                  </div>
                </div>
              </div>
            </details>
          </div>
          <div class="usage-side">
            <strong>{{ formatDurationLong(browser.totalTrackedMs) }}</strong>
            <span class="inline-meta">{{ browser.sessions }} 次访问</span>
          </div>
        </li>
      </ul>
    </section>
    </template>
  </template>
</template>
