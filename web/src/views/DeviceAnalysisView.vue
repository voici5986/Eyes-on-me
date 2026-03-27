<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import { fetchDeviceAnalysis, fetchDeviceDetail, peekDeviceAnalysis, peekDeviceDetail } from "../api";
import { activityHeadline, formatDateTime, formatDurationLong, formatTime, usageShare } from "../lib/activity";
import { ANALYSIS_RANGE_OPTIONS, analysisRangeLabel, normalizeAnalysisRange } from "../lib/analysis-range";
import { buildDeviceUsageTreemap } from "../lib/device-treemap";
import LargeTreemap from "../components/LargeTreemap.vue";
import type { AnalysisRange, DeviceAnalysisResponse, DeviceDetailResponse } from "../types";

const props = defineProps<{
  connection: "connecting" | "live" | "closed";
  nowMs: number;
  refreshToken: number;
}>();

const route = useRoute();
const router = useRouter();
const deviceId = computed(() => String(route.params.deviceId ?? ""));
const selectedRange = computed(() => normalizeAnalysisRange(route.query.range));

const initialDetail = peekDeviceDetail(deviceId.value);
const initialAnalysis = peekDeviceAnalysis(deviceId.value, selectedRange.value);
const loading = ref(!initialDetail || !initialAnalysis);
const error = ref<string | null>(null);
const detail = ref<DeviceDetailResponse | null>(initialDetail);
const analysis = ref<DeviceAnalysisResponse | null>(initialAnalysis);

const appUsage = computed(() => analysis.value?.appUsage ?? []);
const domainUsage = computed(() => analysis.value?.domainUsage ?? []);
const browserUsage = computed(() => analysis.value?.browserUsage ?? []);
const domainPagesByKey = computed(() => {
  const map = new Map<string, DeviceAnalysisResponse["browserUsage"][number]["domains"][number]["pages"]>();

  for (const browser of browserUsage.value) {
    for (const domain of browser.domains) {
      if (!map.has(domain.key)) {
        map.set(domain.key, domain.pages);
      }
    }
  }

  return map;
});
const hasDeviceAnalysis = computed(() =>
  appUsage.value.length > 0 || domainUsage.value.length > 0 || browserUsage.value.length > 0
);
const treemapItems = computed(() => buildDeviceUsageTreemap(analysis.value));

async function loadData(force = false) {
  loading.value = !detail.value || !analysis.value;

  try {
    const [deviceDetail, deviceAnalysis] = await Promise.all([
      fetchDeviceDetail(deviceId.value, force),
      fetchDeviceAnalysis(deviceId.value, selectedRange.value, force)
    ]);
    detail.value = deviceDetail;
    analysis.value = deviceAnalysis;
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
watch(deviceId, () => {
  detail.value = peekDeviceDetail(deviceId.value);
  analysis.value = peekDeviceAnalysis(deviceId.value, selectedRange.value);
  void loadData();
});
watch(selectedRange, () => {
  analysis.value = peekDeviceAnalysis(deviceId.value, selectedRange.value);
  void loadData();
});
watch(() => props.refreshToken, () => {
  void loadData(true);
});
</script>

<template>
  <section v-if="loading" class="panel">
    <p>Loading device analysis...</p>
  </section>

  <section v-else-if="error" class="panel error-panel">
    <p>{{ error }}</p>
  </section>

  <section v-else-if="!detail || !analysis" class="panel">
    <div class="panel-header">
      <h2>设备不存在</h2>
      <RouterLink class="button-link" to="/">返回首页</RouterLink>
    </div>
    <p class="muted">当前设备还没有分析上下文。</p>
  </section>

  <template v-else>
    <section class="page-actions">
      <RouterLink class="button-link" to="/">返回首页</RouterLink>
      <RouterLink class="button-link" :to="`/devices/${encodeURIComponent(deviceId)}`">返回设备明细</RouterLink>
      <span class="muted">连接状态：{{ connection }}</span>
      <span class="muted">统计范围：{{ analysisRangeLabel(selectedRange) }}</span>
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

    <section v-if="!hasDeviceAnalysis" class="panel empty-state">
      <span class="eyebrow">No Breakdown Yet</span>
      <h2 class="analysis-title">{{ detail.device.deviceId }}</h2>
      <p class="analysis-lede">
        当前已经识别到这个设备，但在 {{ analysisRangeLabel(selectedRange) }} 内还没有足够的活动切换来形成可读的应用窗口或域名统计。
        继续使用一段时间后，这里会自动聚合出时长排行。
      </p>
    </section>

    <template v-else>
    <section class="analysis-summary">
      <article class="panel">
        <span class="eyebrow">Device Breakdown</span>
        <h2 class="analysis-title">{{ detail.device.deviceId }}</h2>
        <p class="analysis-lede">
          {{ analysis.latestStatus?.statusText || analysis.currentLabel || activityHeadline(detail.device) }}
          <span class="muted"> · {{ analysisRangeLabel(selectedRange) }}</span>
        </p>

        <div class="stats-row">
          <div class="metric-block">
            <span class="label">活动总时长</span>
            <strong>{{ formatDurationLong(analysis.totalTrackedMs) }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">工作时段活跃</span>
            <strong>{{ formatDurationLong(analysis.workTrackedMs) }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">浏览器时长</span>
            <strong>{{ formatDurationLong(analysis.browserTrackedMs) }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">应用数</span>
            <strong>{{ analysis.appCount }}</strong>
          </div>
        </div>

        <div class="stats-row secondary">
          <div class="metric-block">
            <span class="label">累计记录时长</span>
            <strong>{{ formatDurationLong(analysis.totalTrackedMs) }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">前台切换次数</span>
            <strong>{{ analysis.eventCount }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">最近活动</span>
            <strong>{{ formatDateTime(detail.device.ts) }}</strong>
          </div>
          <div class="metric-block">
            <span class="label">分析生成</span>
            <strong>{{ formatTime(analysis.generatedAt) }}</strong>
          </div>
        </div>
      </article>
    </section>

    <section class="panel">
      <LargeTreemap
        :items="treemapItems"
        title="应用使用版图"
        :subtitle="`${analysisRangeLabel(selectedRange)} 内的应用窗口累计前台时长。浏览器只按应用总时长展示，不混入域名层级。`"
        total-label="累计追踪"
        :height="560"
        :value-formatter="formatDurationLong"
      />
    </section>

    <section class="grid">
      <article class="panel">
        <div class="panel-header">
          <h2>应用窗口累计</h2>
          <span>{{ appUsage.length }}</span>
        </div>

        <ul class="usage-list">
          <li v-for="bucket in appUsage" :key="bucket.key" class="usage-item">
            <div class="usage-copy">
              <strong>{{ bucket.label }}</strong>
              <p>{{ bucket.sublabel || "未提供窗口附加信息" }}</p>
              <div class="usage-bar">
                <span :style="{ width: `${usageShare(analysis.totalTrackedMs, bucket.totalTrackedMs)}%` }" />
              </div>
              <span class="inline-meta">{{ bucket.sessions }} 次进入 · 最近 {{ formatDateTime(bucket.lastSeen) }}</span>
            </div>
            <div class="usage-side">
              <strong>{{ formatDurationLong(bucket.totalTrackedMs) }}</strong>
              <span class="inline-meta">{{ usageShare(analysis.totalTrackedMs, bucket.totalTrackedMs).toFixed(1) }}%</span>
            </div>
          </li>
        </ul>
      </article>

      <article class="panel">
        <div class="panel-header">
          <h2>域名累计</h2>
          <span>{{ domainUsage.length }}</span>
        </div>

        <ul class="usage-list">
          <li v-for="bucket in domainUsage" :key="bucket.key" class="usage-item domain-usage-item">
            <div class="usage-copy">
              <details class="browser-tree domain-tree" :open="bucket.totalTrackedMs === domainUsage[0]?.totalTrackedMs">
                <summary class="browser-tree-summary domain-tree-summary">
                  <div class="domain-tree-body">
                    <div class="domain-tree-main">
                      <div class="domain-tree-heading">
                        <strong>{{ bucket.label }}</strong>
                        <p>{{ bucket.sublabel || "未提供页面标题" }}</p>
                      </div>
                      <div class="domain-tree-stats">
                        <strong>{{ formatDurationLong(bucket.totalTrackedMs) }}</strong>
                        <span class="inline-meta">{{ usageShare(analysis.totalTrackedMs, bucket.totalTrackedMs).toFixed(1) }}%</span>
                      </div>
                    </div>
                    <div class="domain-tree-meta">
                      <span class="inline-meta">页面 {{ (domainPagesByKey.get(bucket.key) ?? []).length }}</span>
                      <span class="inline-meta">访问 {{ bucket.sessions }} 次</span>
                      <span class="inline-meta">最近 {{ formatDateTime(bucket.lastSeen) }}</span>
                    </div>
                  </div>
                </summary>
                <div class="browser-tree-pages">
                  <div
                    v-for="page in domainPagesByKey.get(bucket.key) ?? []"
                    :key="page.key"
                    class="browser-tree-page"
                  >
                    <div class="browser-tree-copy">
                      <strong>{{ page.label }}</strong>
                      <code v-if="page.url" class="url">{{ page.url }}</code>
                    </div>
                    <div class="browser-tree-side">
                      <strong>{{ formatDurationLong(page.totalTrackedMs) }}</strong>
                      <span class="inline-meta">{{ page.sessions }} 次访问</span>
                    </div>
                  </div>
                </div>
              </details>
              <div class="usage-bar">
                <span :style="{ width: `${usageShare(analysis.totalTrackedMs, bucket.totalTrackedMs)}%` }" />
              </div>
            </div>
          </li>
        </ul>
      </article>
    </section>

    <section class="panel">
      <div class="panel-header">
        <h2>浏览器 / 域名 / 页面</h2>
        <span>{{ browserUsage.length }}</span>
      </div>

      <ul class="usage-list">
        <li v-for="browser in browserUsage" :key="browser.key" class="usage-item">
          <div class="usage-copy">
            <strong>{{ browser.label }}</strong>
            <p>{{ browser.family }} · {{ browser.domains.length }} 个域名</p>
            <div class="usage-bar">
              <span :style="{ width: `${usageShare(analysis.totalTrackedMs, browser.totalTrackedMs)}%` }" />
            </div>
            <span class="inline-meta">{{ browser.sessions }} 次访问 · 最近 {{ formatDateTime(browser.lastSeen) }}</span>
            <details
              v-for="domain in browser.domains"
              :key="domain.key"
              class="browser-tree"
            >
              <summary class="browser-tree-summary">
                <span>{{ domain.label }}</span>
                <span>{{ formatDurationLong(domain.totalTrackedMs) }}</span>
              </summary>
              <div class="browser-tree-pages">
                <div
                  v-for="page in domain.pages"
                  :key="page.key"
                  class="browser-tree-page"
                >
                  <div class="browser-tree-copy">
                    <strong>{{ page.label }}</strong>
                    <code v-if="page.url" class="url">{{ page.url }}</code>
                  </div>
                  <div class="browser-tree-side">
                    <strong>{{ formatDurationLong(page.totalTrackedMs) }}</strong>
                    <span class="inline-meta">{{ page.sessions }} 次访问</span>
                  </div>
                </div>
              </div>
            </details>
          </div>
          <div class="usage-side">
            <strong>{{ formatDurationLong(browser.totalTrackedMs) }}</strong>
            <span class="inline-meta">{{ usageShare(analysis.totalTrackedMs, browser.totalTrackedMs).toFixed(1) }}%</span>
          </div>
        </li>
      </ul>
    </section>
    </template>
  </template>
</template>
