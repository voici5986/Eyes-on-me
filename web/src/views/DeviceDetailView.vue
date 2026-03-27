<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute } from "vue-router";
import {
  activityDurationMs,
  activityHeadline,
  activitySubline,
  activityUrl,
  deriveStatusFromActivity,
  formatDuration,
  formatTime,
  isFreshActivity,
  recentActivityDurationMs
} from "../lib/activity";
import { fetchDeviceAnalysis, fetchDeviceDetail, peekDeviceAnalysis, peekDeviceDetail } from "../api";
import { DEFAULT_ANALYSIS_RANGE } from "../lib/analysis-range";
import type { DeviceAnalysisResponse, DeviceDetailResponse } from "../types";

const props = defineProps<{
  connection: "connecting" | "live" | "closed";
  nowMs: number;
  refreshToken: number;
}>();

const route = useRoute();

const deviceId = computed(() => String(route.params.deviceId ?? ""));
const initialDetail = peekDeviceDetail(deviceId.value);
const initialAnalysis = peekDeviceAnalysis(deviceId.value, DEFAULT_ANALYSIS_RANGE);
const loading = ref(!initialDetail || !initialAnalysis);
const error = ref<string | null>(null);
const detail = ref<DeviceDetailResponse | null>(initialDetail);
const analysis = ref<DeviceAnalysisResponse | null>(initialAnalysis);

const currentDevice = computed(() => detail.value?.device ?? null);
const recentActivities = computed(() => detail.value?.recentActivities ?? []);
const latestStatus = computed(() => detail.value?.latestStatus ?? null);

async function loadData(force = false) {
  loading.value = !detail.value || !analysis.value;

  try {
    const [deviceDetail, deviceAnalysis] = await Promise.all([
      fetchDeviceDetail(deviceId.value, force),
      fetchDeviceAnalysis(deviceId.value, DEFAULT_ANALYSIS_RANGE, force)
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

onMounted(() => {
  if (loading.value) {
    void loadData();
  }
});
watch(deviceId, () => {
  detail.value = peekDeviceDetail(deviceId.value);
  analysis.value = peekDeviceAnalysis(deviceId.value, DEFAULT_ANALYSIS_RANGE);
  void loadData();
});
watch(() => props.refreshToken, () => {
  void loadData(true);
});
</script>

<template>
  <section v-if="loading" class="panel">
    <p>Loading device detail...</p>
  </section>

  <section v-else-if="error" class="panel error-panel">
    <p>{{ error }}</p>
  </section>

  <section v-else-if="!currentDevice" class="panel">
    <div class="panel-header">
      <h2>设备不存在</h2>
      <RouterLink class="button-link" to="/">返回首页</RouterLink>
    </div>
    <p class="muted">当前设备还没有上报活动，或者设备 ID 已变化。</p>
  </section>

  <template v-else>
    <section class="page-actions">
      <RouterLink class="button-link" to="/">返回首页</RouterLink>
      <RouterLink class="button-link" :to="`/devices/${encodeURIComponent(deviceId)}/analysis?range=${DEFAULT_ANALYSIS_RANGE}`">设备分析页</RouterLink>
      <span class="muted">连接状态：{{ connection }}</span>
    </section>

    <section class="grid">
      <article class="panel">
        <div class="panel-header">
          <h2>当前设备</h2>
          <span>{{ currentDevice.platform }}</span>
        </div>

        <div class="status-detail">
          <strong>{{ currentDevice.deviceId }}</strong>
          <p>{{ activityHeadline(currentDevice) }}</p>
          <span>{{ currentDevice.app.name }}</span>
          <span>
            {{
              isFreshActivity(currentDevice, props.nowMs)
                ? `最近上报 ${formatDuration(activityDurationMs(currentDevice, props.nowMs))} 前`
                : "当前状态已过期"
            }}
          </span>
          <span>最后更新 {{ formatTime(currentDevice.ts) }}</span>
          <code v-if="activityUrl(currentDevice)" class="url">{{ activityUrl(currentDevice) }}</code>
        </div>
      </article>

      <article class="panel">
        <div class="panel-header">
          <h2>自动状态</h2>
        </div>

        <div class="status-detail">
          <strong>{{ latestStatus?.statusText || deriveStatusFromActivity(currentDevice) }}</strong>
          <span>{{ currentDevice.deviceId }} / {{ currentDevice.platform }}</span>
          <span>{{ formatTime((latestStatus || currentDevice).ts) }}</span>
        </div>
      </article>
    </section>

    <section class="panel">
      <div class="panel-header">
        <h2>最近活动</h2>
        <span>{{ recentActivities.length }}</span>
      </div>

      <ul class="list">
        <li v-for="(activity, index) in recentActivities" :key="activity.eventId" class="list-item">
          <div>
            <strong>{{ activityHeadline(activity) }}</strong>
            <p>{{ activitySubline(activity) }}</p>
            <code v-if="activityUrl(activity)" class="url">{{ activityUrl(activity) }}</code>
          </div>
          <div class="meta">
            <span>{{ activity.app.name }}</span>
            <span>
              {{
                index === 0
                  ? (isFreshActivity(activity, props.nowMs) ? "进行中" : "上次上报")
                  : "持续 " + formatDuration(recentActivityDurationMs(recentActivities, activity, index, props.nowMs))
              }}
            </span>
            <span>{{ formatTime(activity.ts) }}</span>
          </div>
        </li>
      </ul>
    </section>

    <section class="panel">
      <div class="panel-header">
        <h2>分析预览</h2>
        <span>{{ (analysis?.appUsage.length ?? 0) + (analysis?.domainUsage.length ?? 0) + (analysis?.browserUsage.length ?? 0) }}</span>
      </div>
      <div class="placeholder-stack">
        <div
          v-if="(analysis?.appUsage.length ?? 0) === 0 && (analysis?.domainUsage.length ?? 0) === 0 && (analysis?.browserUsage.length ?? 0) === 0"
          class="placeholder-card"
        >
          <strong>这个设备还没有统计结果</strong>
          <p>说明这个设备目前只有极少活动，或者 agent 还没往当前这套服务里持续上报。</p>
        </div>
        <div v-for="bucket in analysis?.appUsage.slice(0, 3) ?? []" :key="bucket.key" class="placeholder-card">
          <strong>{{ bucket.label }}</strong>
          <p>{{ bucket.sublabel || "应用窗口累计时长" }}</p>
          <span class="inline-meta">累计 {{ formatDuration(bucket.totalTrackedMs) }}</span>
        </div>
        <div v-for="bucket in analysis?.domainUsage.slice(0, 2) ?? []" :key="bucket.key" class="placeholder-card">
          <strong>{{ bucket.label }}</strong>
          <p>{{ bucket.sublabel || "浏览器域名累计时长" }}</p>
          <span class="inline-meta">累计 {{ formatDuration(bucket.totalTrackedMs) }}</span>
        </div>
        <div v-for="browser in analysis?.browserUsage.slice(0, 2) ?? []" :key="browser.key" class="placeholder-card">
          <strong>{{ browser.label }}</strong>
          <p>{{ browser.domains[0]?.label || "浏览器层级分析" }}</p>
          <span class="inline-meta">累计 {{ formatDuration(browser.totalTrackedMs) }}</span>
        </div>
      </div>
    </section>
  </template>
</template>
