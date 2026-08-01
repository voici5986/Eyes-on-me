<script setup lang="ts">
import { ArrowLeft, ArrowRight, Clock3, Globe2, Layers3, Monitor, Moon, TimerReset } from "@lucide/vue";
import { computed, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import { fetchDeviceAnalysis, fetchDeviceDetail, peekDeviceAnalysis, peekDeviceDetail } from "../api";
import AppDetailDrawer from "../components/AppDetailDrawer.vue";
import CategoryBreakdown from "../components/CategoryBreakdown.vue";
import DailyTrend from "../components/DailyTrend.vue";
import HourlyActivityChart from "../components/HourlyActivityChart.vue";
import LargeTreemap from "../components/LargeTreemap.vue";
import RangeSwitcher from "../components/RangeSwitcher.vue";
import type { TreemapNodeInput } from "../components/treemap";
import { activityHeadline, deriveStatusFromActivity, formatDateTime, formatDurationLong, usageShare } from "../lib/activity";
import { analysisRangeLabel, normalizeAnalysisRange } from "../lib/analysis-range";
import { buildDeviceUsageTreemap } from "../lib/device-treemap";
import type { AnalysisRange, AppUsageBucket, DeviceAnalysisResponse, DeviceDetailResponse } from "../types";

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
const selectedApp = ref<AppUsageBucket | null>(null);

const appUsage = computed(() => analysis.value?.appUsage ?? []);
const browserUsage = computed(() => analysis.value?.browserUsage ?? []);
const treemapItems = computed(() => buildDeviceUsageTreemap(analysis.value));

async function loadData(force = false) {
  loading.value = !detail.value || !analysis.value;
  try {
    const [nextDetail, nextAnalysis] = await Promise.all([
      fetchDeviceDetail(deviceId.value, force),
      fetchDeviceAnalysis(deviceId.value, selectedRange.value, force)
    ]);
    detail.value = nextDetail;
    analysis.value = nextAnalysis;
    error.value = null;
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
  }
}

async function updateRange(range: AnalysisRange) {
  if (range !== selectedRange.value) await router.replace({ query: { ...route.query, range } });
}

function selectTreemapNode(node: TreemapNodeInput) {
  selectedApp.value = appUsage.value.find((app) => app.key === node.id) ?? null;
}

onMounted(() => {
  if (loading.value) void loadData();
});
watch(deviceId, () => {
  detail.value = peekDeviceDetail(deviceId.value);
  analysis.value = peekDeviceAnalysis(deviceId.value, selectedRange.value);
  void loadData();
});
watch(selectedRange, () => {
  analysis.value = peekDeviceAnalysis(deviceId.value, selectedRange.value);
  selectedApp.value = null;
  void loadData();
});
watch(() => props.refreshToken, () => void loadData(true));
</script>

<template>
  <div class="page-stack">
    <header class="page-heading device-page-heading">
      <div class="page-heading__identity">
        <RouterLink class="icon-button" to="/" aria-label="返回活动概览" title="返回活动概览"><ArrowLeft :size="19" /></RouterLink>
        <span class="device-avatar"><Monitor :size="22" /></span>
        <div>
          <span class="section-kicker">Device analysis</span>
          <h1>{{ deviceId }}</h1>
          <p v-if="detail && analysis">{{ deriveStatusFromActivity(detail.device) }} · {{ analysisRangeLabel(selectedRange) }}</p>
        </div>
      </div>
      <div class="page-heading__tools">
        <RouterLink class="text-button" :to="`/devices/${encodeURIComponent(deviceId)}`">查看时间线</RouterLink>
        <RangeSwitcher :model-value="selectedRange" @update:model-value="updateRange" />
      </div>
    </header>

    <div v-if="loading" class="state-view"><span class="spinner" /> 正在加载设备分析</div>
    <div v-else-if="error" class="state-view is-error">{{ error }}</div>
    <div v-else-if="!detail || !analysis" class="state-view"><strong>设备不存在</strong></div>
    <div v-else-if="analysis.totalTrackedMs === 0" class="state-view">
      <Clock3 :size="24" /><strong>当前范围没有可分析活动</strong>
    </div>

    <template v-else-if="detail && analysis">
      <section class="current-activity-bar" aria-label="当前活动">
        <span><Monitor :size="18" /></span>
        <div><small>当前活动</small><strong>{{ deriveStatusFromActivity(detail.device) }}</strong></div>
        <time>{{ formatDateTime(detail.device.ts) }}</time>
      </section>

      <section class="metric-grid metric-grid--six">
        <article class="metric-card is-primary">
          <span><Clock3 :size="16" /> 活动时长</span>
          <strong>{{ formatDurationLong(analysis.totalTrackedMs) }}</strong>
          <small>{{ analysis.eventCount }} 次前台切换</small>
        </article>
        <article class="metric-card">
          <span><TimerReset :size="16" /> 工作时段</span>
          <strong>{{ formatDurationLong(analysis.workTrackedMs) }}</strong>
          <small>{{ usageShare(analysis.totalTrackedMs, analysis.workTrackedMs).toFixed(0) }}%</small>
        </article>
        <article class="metric-card">
          <span><Moon :size="16" /> 非工作时段</span>
          <strong>{{ formatDurationLong(analysis.afterHoursTrackedMs) }}</strong>
          <small>活跃统计</small>
        </article>
        <article class="metric-card">
          <span><Globe2 :size="16" /> 浏览器</span>
          <strong>{{ formatDurationLong(analysis.browserTrackedMs) }}</strong>
          <small>{{ analysis.domainUsage.length }} 个主要域名</small>
        </article>
        <article class="metric-card">
          <span><Layers3 :size="16" /> 应用</span>
          <strong>{{ analysis.appCount }}</strong>
          <small>{{ appUsage.reduce((sum, app) => sum + app.windows.length, 0) }} 个窗口 / Tab</small>
        </article>
        <article class="metric-card">
          <span><Monitor :size="16" /> 离开设备</span>
          <strong>{{ formatDurationLong(analysis.idleTrackedMs + analysis.lockedTrackedMs) }}</strong>
          <small>锁屏 {{ formatDurationLong(analysis.lockedTrackedMs) }}</small>
        </article>
      </section>

      <section class="dashboard-grid">
        <article class="data-section is-wide">
          <header class="section-heading">
            <div><span class="section-kicker">Rhythm</span><h2>24 小时活动分布</h2></div>
            <span>{{ formatDateTime(analysis.generatedAt) }}</span>
          </header>
          <HourlyActivityChart :buckets="analysis.hourlyUsage" />
        </article>
        <article class="data-section">
          <header class="section-heading">
            <div><span class="section-kicker">Categories</span><h2>活动分类</h2></div>
          </header>
          <CategoryBreakdown :items="analysis.categoryUsage" :total="analysis.totalTrackedMs" />
        </article>
      </section>

      <section v-if="analysis.dailyUsage.length > 1" class="data-section">
        <header class="section-heading"><div><span class="section-kicker">Trend</span><h2>逐日趋势</h2></div></header>
        <DailyTrend :items="analysis.dailyUsage" />
      </section>

      <section class="data-section treemap-section">
        <LargeTreemap
          :items="treemapItems"
          title="应用使用版图"
          subtitle="按应用累计的前台活跃时长"
          total-label="累计活跃"
          :height="520"
          :value-formatter="formatDurationLong"
          @select="selectTreemapNode"
        />
      </section>

      <section class="dashboard-grid lower-grid">
        <article class="data-section is-wide">
          <header class="section-heading">
            <div><span class="section-kicker">Applications</span><h2>应用与窗口</h2></div>
            <span>{{ appUsage.length }} 个应用</span>
          </header>
          <div class="rank-list">
            <button v-for="(app, index) in appUsage" :key="app.key" type="button" class="rank-row" @click="selectedApp = app">
              <span class="rank-row__index">{{ String(index + 1).padStart(2, "0") }}</span>
              <span class="app-symbol">{{ app.label.slice(0, 1).toUpperCase() }}</span>
              <span class="rank-row__main">
                <strong>{{ app.label }}</strong>
                <small>{{ app.windows.length }} 个窗口 / Tab · {{ app.sessions }} 次进入</small>
                <i><span :style="{ width: `${usageShare(analysis.totalTrackedMs, app.totalTrackedMs)}%` }" /></i>
              </span>
              <span class="rank-row__value"><strong>{{ formatDurationLong(app.totalTrackedMs) }}</strong><small>{{ usageShare(analysis.totalTrackedMs, app.totalTrackedMs).toFixed(1) }}%</small></span>
              <ArrowRight :size="17" class="rank-row__arrow" />
            </button>
          </div>
        </article>

        <article class="data-section">
          <header class="section-heading">
            <div><span class="section-kicker">Web</span><h2>浏览器站点</h2></div>
            <span>{{ browserUsage.length }} 个浏览器</span>
          </header>
          <div class="browser-list">
            <details v-for="browser in browserUsage" :key="browser.key" class="browser-item">
              <summary>
                <span><strong>{{ browser.label }}</strong><small>{{ browser.domains.length }} 个域名</small></span>
                <span><strong>{{ formatDurationLong(browser.totalTrackedMs) }}</strong><ArrowRight :size="15" /></span>
              </summary>
              <div class="browser-item__domains">
                <details v-for="domain in browser.domains" :key="domain.key">
                  <summary><span>{{ domain.label }}</span><strong>{{ formatDurationLong(domain.totalTrackedMs) }}</strong></summary>
                  <div v-for="page in domain.pages" :key="page.key" class="page-row">
                    <span><strong>{{ page.label }}</strong><small v-if="page.url">{{ page.url }}</small></span>
                    <strong>{{ formatDurationLong(page.totalTrackedMs) }}</strong>
                  </div>
                </details>
              </div>
            </details>
          </div>
        </article>
      </section>
    </template>
  </div>

  <AppDetailDrawer :app="selectedApp" @close="selectedApp = null" />
</template>
