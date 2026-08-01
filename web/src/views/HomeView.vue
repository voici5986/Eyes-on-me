<script setup lang="ts">
import { AppWindow, ArrowRight, Clock3, Globe2, Laptop2, Moon, TimerReset } from "@lucide/vue";
import { computed, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import { fetchAnalysisOverview, fetchDevices, peekAnalysisOverview, peekDevices } from "../api";
import AppDetailDrawer from "../components/AppDetailDrawer.vue";
import CategoryBreakdown from "../components/CategoryBreakdown.vue";
import DailyTrend from "../components/DailyTrend.vue";
import HourlyActivityChart from "../components/HourlyActivityChart.vue";
import RangeSwitcher from "../components/RangeSwitcher.vue";
import {
  activityHeadline,
  deriveStatusFromActivity,
  formatDateTime,
  formatDurationLong,
  isFreshActivity,
  usageShare
} from "../lib/activity";
import { analysisRangeLabel, normalizeAnalysisRange } from "../lib/analysis-range";
import type { AnalysisOverviewResponse, AnalysisRange, AppUsageBucket, DevicesResponse } from "../types";

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
const analysis = ref<AnalysisOverviewResponse | null>(initialAnalysis);
const devicesResponse = ref<DevicesResponse | null>(initialDevices);
const selectedApp = ref<AppUsageBucket | null>(null);

const devicesById = computed(() => new Map((devicesResponse.value?.devices ?? []).map((item) => [item.device.deviceId, item])));
const topApps = computed(() => analysis.value?.topAppUsage ?? []);
const topDomains = computed(() => analysis.value?.topDomainUsage ?? []);
const isEmpty = computed(() => !analysis.value || analysis.value.totalTrackedMs === 0);
const deviceRows = computed(() =>
  (analysis.value?.devices ?? []).map((item) => {
    const overview = devicesById.value.get(item.deviceId);
    return {
      ...item,
      recording: overview?.recording ?? null,
      headline: overview?.device ? activityHeadline(overview.device) : item.currentLabel,
      statusText: overview?.recording && !overview.recording.desiredEnabled ? "采集已暂停" : overview?.device ? deriveStatusFromActivity(overview.device) : `正在使用 ${item.currentLabel}`,
      fresh: overview?.recording?.desiredEnabled !== false && overview?.device ? isFreshActivity(overview.device, props.nowMs) : false,
      appName: overview?.device.app.name ?? "暂无应用"
    };
  })
);

async function loadData(force = false) {
  loading.value = !analysis.value || !devicesResponse.value;
  try {
    const [nextAnalysis, nextDevices] = await Promise.all([
      fetchAnalysisOverview(selectedRange.value, force),
      fetchDevices(force)
    ]);
    analysis.value = nextAnalysis;
    devicesResponse.value = nextDevices;
    error.value = null;
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
  }
}

async function updateRange(range: AnalysisRange) {
  if (range !== selectedRange.value) {
    await router.replace({ query: { ...route.query, range } });
  }
}

onMounted(() => {
  if (loading.value) void loadData();
});
watch(selectedRange, () => {
  analysis.value = peekAnalysisOverview(selectedRange.value);
  void loadData();
});
watch(() => props.refreshToken, () => void loadData(true));
</script>

<template>
  <div class="page-stack">
    <header class="page-heading">
      <div>
        <span class="section-kicker">Overview</span>
        <h1>活动概览</h1>
        <p v-if="analysis">{{ analysisRangeLabel(selectedRange) }} · 更新于 {{ formatDateTime(analysis.generatedAt) }}</p>
      </div>
      <RangeSwitcher :model-value="selectedRange" @update:model-value="updateRange" />
    </header>

    <div v-if="loading" class="state-view"><span class="spinner" /> 正在整理活动数据</div>
    <div v-else-if="error" class="state-view is-error">{{ error }}</div>
    <div v-else-if="isEmpty" class="state-view">
      <Clock3 :size="24" />
      <strong>这个时间范围还没有活动</strong>
      <span>客户端开始上报后，这里会自动出现统计。</span>
    </div>

    <template v-else-if="analysis">
      <section class="metric-grid" aria-label="核心统计">
        <article class="metric-card is-primary">
          <span><Clock3 :size="16" /> 活动时长</span>
          <strong>{{ formatDurationLong(analysis.totalTrackedMs) }}</strong>
          <small>{{ analysis.deviceCount }} 台设备参与统计</small>
        </article>
        <article class="metric-card">
          <span><TimerReset :size="16" /> 工作时段</span>
          <strong>{{ formatDurationLong(analysis.workTrackedMs) }}</strong>
          <small>{{ usageShare(analysis.totalTrackedMs, analysis.workTrackedMs).toFixed(0) }}% 的活跃时间</small>
        </article>
        <article class="metric-card">
          <span><Globe2 :size="16" /> 浏览器</span>
          <strong>{{ formatDurationLong(analysis.browserTrackedMs) }}</strong>
          <small>{{ analysis.topDomainUsage.length }} 个主要域名</small>
        </article>
        <article class="metric-card">
          <span><Moon :size="16" /> 非工作时段</span>
          <strong>{{ formatDurationLong(analysis.afterHoursTrackedMs) }}</strong>
          <small>空闲 {{ formatDurationLong(analysis.idleTrackedMs) }}</small>
        </article>
        <article class="metric-card">
          <span><AppWindow :size="16" /> 应用</span>
          <strong>{{ analysis.appCount }}</strong>
          <small>{{ analysis.topAppUsage.reduce((sum, item) => sum + item.windows.length, 0) }} 个窗口 / Tab</small>
        </article>
      </section>

      <section class="dashboard-grid">
        <article class="data-section is-wide">
          <header class="section-heading">
            <div><span class="section-kicker">Rhythm</span><h2>24 小时活动分布</h2></div>
            <span>本地时间</span>
          </header>
          <HourlyActivityChart :buckets="analysis.hourlyUsage" />
        </article>

        <article class="data-section">
          <header class="section-heading">
            <div><span class="section-kicker">Categories</span><h2>活动分类</h2></div>
            <span>{{ analysis.categoryUsage.length }} 类</span>
          </header>
          <CategoryBreakdown :items="analysis.categoryUsage" :total="analysis.totalTrackedMs" />
        </article>
      </section>

      <section v-if="analysis.dailyUsage.length > 1" class="data-section">
        <header class="section-heading">
          <div><span class="section-kicker">Trend</span><h2>逐日趋势</h2></div>
          <span>{{ analysis.dailyUsage.length }} 天</span>
        </header>
        <DailyTrend :items="analysis.dailyUsage" />
      </section>

      <section class="dashboard-grid lower-grid">
        <article class="data-section is-wide">
          <header class="section-heading">
            <div><span class="section-kicker">Applications</span><h2>应用使用</h2></div>
            <span>{{ topApps.length }} 个应用</span>
          </header>
          <div class="rank-list">
            <button v-for="(app, index) in topApps" :key="app.key" type="button" class="rank-row" @click="selectedApp = app">
              <span class="rank-row__index">{{ String(index + 1).padStart(2, "0") }}</span>
              <span class="app-symbol">{{ app.label.slice(0, 1).toUpperCase() }}</span>
              <span class="rank-row__main">
                <strong>{{ app.label }}</strong>
                <small>{{ app.windows.length }} 个窗口 / Tab · {{ app.sessions }} 次进入</small>
                <i><span :style="{ width: `${usageShare(analysis.totalTrackedMs, app.totalTrackedMs)}%` }" /></i>
              </span>
              <span class="rank-row__value">
                <strong>{{ formatDurationLong(app.totalTrackedMs) }}</strong>
                <small>{{ usageShare(analysis.totalTrackedMs, app.totalTrackedMs).toFixed(1) }}%</small>
              </span>
              <ArrowRight :size="17" class="rank-row__arrow" />
            </button>
          </div>
        </article>

        <article class="data-section">
          <header class="section-heading">
            <div><span class="section-kicker">Devices</span><h2>设备状态</h2></div>
            <span>{{ deviceRows.length }} 台</span>
          </header>
          <div class="device-list">
            <article v-for="device in deviceRows" :key="device.deviceId" class="device-row">
              <span class="device-row__icon"><Laptop2 :size="18" /></span>
              <div>
                <strong>{{ device.deviceId }}</strong>
                <p class="device-row__status">{{ device.statusText }}</p>
                <small><i :class="{ 'is-live': device.fresh }" /> {{ device.recording?.desiredEnabled === false ? "暂停" : device.recording?.appliedEnabled === device.recording?.desiredEnabled ? "采集中" : "等待 Agent" }} · {{ device.headline }} · {{ formatDurationLong(device.totalTrackedMs) }}</small>
              </div>
              <div class="device-row__actions">
                <RouterLink :to="`/devices/${encodeURIComponent(device.deviceId)}`" title="查看时间线">明细</RouterLink>
                <RouterLink :to="`/devices/${encodeURIComponent(device.deviceId)}/analysis?range=${selectedRange}`" title="查看设备分析">分析</RouterLink>
              </div>
            </article>
          </div>

          <div v-if="topDomains.length" class="domain-summary">
            <h3>主要域名</h3>
            <div v-for="domain in topDomains.slice(0, 5)" :key="domain.key">
              <span>{{ domain.label }}</span>
              <strong>{{ formatDurationLong(domain.totalTrackedMs) }}</strong>
            </div>
          </div>
        </article>
      </section>
    </template>
  </div>

  <AppDetailDrawer :app="selectedApp" @close="selectedApp = null" />
</template>
