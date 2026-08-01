<script setup lang="ts">
import { ArrowLeft, BarChart3, ChevronLeft, ChevronRight, CirclePause, CirclePlay, Clock3, Download, Image, Laptop2, Lock, Monitor, RefreshCw, ScanText, Search, TimerReset, Trash2, X } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { RouterLink, useRoute } from "vue-router";
import { deleteActivities, deleteActivity, deleteScreenshot, fetchDeviceAnalysis, fetchDeviceDetail, fetchTimeline, fetchWorkSessions, peekDeviceAnalysis, peekDeviceDetail, retryScreenshotOcr, setDeviceRecording } from "../api";
import {
  activityDurationMs,
  activityHeadline,
  activitySubline,
  activityUrl,
  deriveStatusFromActivity,
  formatDateTime,
  formatDuration,
  formatDurationLong,
  isFreshActivity
} from "../lib/activity";
import { DEFAULT_ANALYSIS_RANGE } from "../lib/analysis-range";
import type { ActivityEvent, DeviceAnalysisResponse, DeviceDetailResponse, ScreenshotRecord, TimelineItem, TimelineResponse, WorkSession } from "../types";

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
const searchQuery = ref("");
const timeline = ref<TimelineResponse | null>(null);
const searching = ref(false);
const searchError = ref<string | null>(null);
const selectedScreenshot = ref<ScreenshotRecord | null>(null);
const actionBusy = ref<string | null>(null);
const selectedDate = ref(localDate());
const appFilter = ref("");
const domainFilter = ref("");
const categoryFilter = ref("");
const startTime = ref("");
const endTime = ref("");
const pageOffset = ref(0);
const pageSize = 100;
const sessions = ref<WorkSession[]>([]);

const currentDevice = computed(() => detail.value?.device ?? null);
const currentStatus = computed(() => deriveStatusFromActivity(currentDevice.value));
const shownActivities = computed<TimelineItem[]>(() => timeline.value?.items ?? []);
const recording = computed(() => detail.value?.recording ?? null);

function localDate() {
  const now = new Date();
  return new Date(now.getTime() - now.getTimezoneOffset() * 60_000).toISOString().slice(0, 10);
}

async function loadData(force = false) {
  loading.value = !detail.value || !analysis.value;
  try {
    const [nextDetail, nextAnalysis, nextTimeline, nextSessions] = await Promise.all([
      fetchDeviceDetail(deviceId.value, force),
      fetchDeviceAnalysis(deviceId.value, DEFAULT_ANALYSIS_RANGE, force),
      fetchTimeline({
        date: selectedDate.value,
        deviceId: deviceId.value,
        app: appFilter.value || undefined,
        domain: domainFilter.value || undefined,
        category: categoryFilter.value || undefined,
        q: searchQuery.value || undefined,
        limit: pageSize,
        offset: pageOffset.value
      }),
      fetchWorkSessions({ date: selectedDate.value, deviceId: deviceId.value })
    ]);
    detail.value = nextDetail;
    analysis.value = nextAnalysis;
    timeline.value = nextTimeline;
    sessions.value = nextSessions.sessions;
    error.value = null;
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
  }
}

async function submitSearch() {
  pageOffset.value = 0;
  searching.value = true;
  try {
    await loadData(true);
    searchError.value = null;
  } catch (err) {
    searchError.value = err instanceof Error ? err.message : String(err);
  } finally {
    searching.value = false;
  }
}

function clearSearch() {
  searchQuery.value = "";
  searchError.value = null;
  pageOffset.value = 0;
  void loadData(true);
}

function closeScreenshot() { selectedScreenshot.value = null; }
async function removeActivity(activity: ActivityEvent) {
  if (!window.confirm(`删除 ${formatDateTime(activity.ts)} 的活动及关联截图？`)) return;
  actionBusy.value = activity.eventId;
  try { await deleteActivity(activity.eventId); if (selectedScreenshot.value?.eventId === activity.eventId) closeScreenshot(); await loadData(true); }
  catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { actionBusy.value = null; }
}
async function removeScreenshot(item: ScreenshotRecord) {
  if (!window.confirm("删除这张截图？活动记录会保留。")) return;
  actionBusy.value = item.id;
  try { await deleteScreenshot(item.id); closeScreenshot(); await loadData(true); }
  catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { actionBusy.value = null; }
}
async function toggleRecording() {
  if (!recording.value) return;
  const enabled = !recording.value.desiredEnabled;
  actionBusy.value = "recording";
  try {
    const state = await setDeviceRecording(deviceId.value, enabled);
    if (detail.value) detail.value.recording = state;
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { actionBusy.value = null; }
}
function selectionRange() {
  if (!startTime.value && !endTime.value) return { date: selectedDate.value };
  if (!startTime.value || !endTime.value) throw new Error("开始时间和结束时间需要同时填写");
  const start = new Date(`${selectedDate.value}T${startTime.value}:00`);
  const end = new Date(`${selectedDate.value}T${endTime.value}:00`);
  if (!(end > start)) throw new Error("结束时间必须晚于开始时间");
  return { start: start.toISOString(), end: end.toISOString() };
}
async function removeSelection() {
  const scope = [appFilter.value && `应用 ${appFilter.value}`, domainFilter.value && `域名 ${domainFilter.value}`, categoryFilter.value && `分类 ${categoryFilter.value}`].filter(Boolean).join("、") || "全部活动";
  if (!window.confirm(`删除 ${selectedDate.value} 的${scope}及关联截图？此操作会使日报和记忆失效。`)) return;
  actionBusy.value = "bulk-delete";
  try {
    const result = await deleteActivities({ ...selectionRange(), deviceId: deviceId.value, app: appFilter.value || undefined, domain: domainFilter.value || undefined, category: categoryFilter.value || undefined });
    pageOffset.value = 0;
    await loadData(true);
    searchError.value = `已删除 ${result.deletedActivities} 条活动和 ${result.deletedScreenshots} 张截图`;
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { actionBusy.value = null; }
}
async function changePage(nextOffset: number) {
  pageOffset.value = Math.max(0, nextOffset);
  await loadData(true);
}
async function retryOcr(item: ScreenshotRecord) {
  actionBusy.value = item.id;
  try {
    const updated = await retryScreenshotOcr(item.id);
    screenshots.value = screenshots.value.map((value) => value.id === item.id ? updated : value);
    selectedScreenshot.value = updated;
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { actionBusy.value = null; }
}
function handleKeydown(event: KeyboardEvent) { if (event.key === "Escape") closeScreenshot(); }

onMounted(() => { window.addEventListener("keydown", handleKeydown); void loadData(); });
onBeforeUnmount(() => window.removeEventListener("keydown", handleKeydown));
watch(deviceId, () => {
  detail.value = peekDeviceDetail(deviceId.value);
  analysis.value = peekDeviceAnalysis(deviceId.value, DEFAULT_ANALYSIS_RANGE);
    clearSearch();
    timeline.value = null;
    sessions.value = [];
    closeScreenshot();
  void loadData();
});
watch(() => props.refreshToken, () => void loadData(true));
watch(selectedDate, () => { pageOffset.value = 0; void loadData(true); });
</script>

<template>
  <div class="page-stack">
    <header class="page-heading device-page-heading">
      <div class="page-heading__identity">
        <RouterLink class="icon-button" to="/" aria-label="返回活动概览" title="返回活动概览"><ArrowLeft :size="19" /></RouterLink>
        <span class="device-avatar"><Laptop2 :size="22" /></span>
        <div>
          <span class="section-kicker">Device timeline</span>
          <h1>{{ deviceId }}</h1>
          <p v-if="currentDevice">{{ currentDevice.platform }} · {{ isFreshActivity(currentDevice, props.nowMs) ? "正在上报" : "上次活动已结束" }}</p>
          <p v-if="currentDevice" class="page-heading__current-status">{{ currentStatus }}</p>
        </div>
      </div>
      <div class="page-heading__tools"><button class="text-button" :class="{ 'is-danger': recording?.desiredEnabled }" type="button" :disabled="actionBusy === 'recording'" @click="toggleRecording"><component :is="recording?.desiredEnabled ? CirclePause : CirclePlay" :size="15" />{{ recording?.desiredEnabled ? "暂停采集" : "恢复采集" }}</button><a class="text-button" :href="`/api/export/activities?format=csv&deviceId=${encodeURIComponent(deviceId)}&date=${selectedDate}`" download><Download :size="15" />导出</a><RouterLink class="text-button" :to="`/devices/${encodeURIComponent(deviceId)}/analysis?range=${DEFAULT_ANALYSIS_RANGE}`"><BarChart3 :size="15" /> 设备分析</RouterLink></div>
    </header>

    <div v-if="loading" class="state-view"><span class="spinner" /> 正在加载设备活动</div>
    <div v-else-if="error" class="state-view is-error">{{ error }}</div>
    <div v-else-if="!currentDevice" class="state-view"><strong>设备不存在</strong></div>

    <template v-else>
      <section class="metric-grid metric-grid--device">
        <article class="metric-card is-primary">
          <span><Monitor :size="16" /> 当前活动</span>
          <strong class="metric-card__text">{{ activityHeadline(currentDevice) }}</strong>
          <small class="metric-card__status">{{ currentStatus }}</small>
        </article>
        <article class="metric-card">
          <span><Clock3 :size="16" /> 今日活跃</span>
          <strong>{{ formatDurationLong(analysis?.totalTrackedMs ?? 0) }}</strong>
          <small>{{ analysis?.eventCount ?? 0 }} 次前台切换</small>
        </article>
        <article class="metric-card">
          <span><TimerReset :size="16" /> 最近上报</span>
          <strong>{{ isFreshActivity(currentDevice, props.nowMs) ? formatDuration(activityDurationMs(currentDevice, props.nowMs)) + " 前" : "已过期" }}</strong>
          <small>{{ formatDateTime(currentDevice.ts) }}</small>
        </article>
        <article class="metric-card">
          <span><Lock :size="16" /> 当前状态</span>
          <strong>{{ recording?.desiredEnabled ? (currentDevice.presence === "active" ? "活跃" : currentDevice.presence === "idle" ? "空闲" : "锁屏") : "采集已暂停" }}</strong>
          <small>{{ recording?.appliedEnabled === recording?.desiredEnabled ? "Agent 已执行" : "等待 Agent 确认" }} · {{ connection === "live" ? "实时连接正常" : "实时连接断开" }}</small>
        </article>
      </section>

      <section v-if="sessions.length" class="data-section session-section">
        <header class="section-heading"><div><span class="section-kicker">Sessions</span><h2>连续工作片段</h2></div><span>{{ sessions.length }} 段</span></header>
        <div class="session-list">
          <details v-for="session in sessions" :key="session.id" class="session-row">
            <summary><span><strong>{{ new Date(session.startedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) }}–{{ new Date(session.endedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) }}</strong><small>{{ session.summary }}</small></span><span>{{ formatDurationLong(session.totalTrackedMs) }}</span></summary>
            <div class="session-row__detail"><div><span v-for="app in session.appNames" :key="app">{{ app }}</span></div><ul v-if="session.potentialTodos.length"><li v-for="todo in session.potentialTodos" :key="`${todo.sourceEventId}-${todo.text}`">{{ todo.text }}</li></ul></div>
          </details>
        </div>
      </section>

      <section class="data-section timeline-section">
        <header class="section-heading timeline-heading">
          <div><span class="section-kicker">Timeline</span><h2>{{ selectedDate }} 活动</h2><p>{{ timeline?.total ?? 0 }} 条记录</p></div>
          <form class="activity-search" role="search" @submit.prevent="submitSearch">
            <Search :size="15" />
            <input v-model="searchQuery" type="search" placeholder="搜索窗口、域名或 URL" aria-label="搜索设备活动" />
            <button v-if="searchQuery" type="button" class="activity-search__clear" aria-label="清除搜索" title="清除" @click="clearSearch"><X :size="14" /></button>
            <button type="submit" class="activity-search__submit" :disabled="searching">{{ searching ? "搜索中" : "搜索" }}</button>
          </form>
        </header>

        <div class="timeline-filters">
          <label><span>日期</span><input v-model="selectedDate" type="date" /></label>
          <label><span>应用</span><input v-model="appFilter" type="text" placeholder="名称或 Bundle ID" /></label>
          <label><span>域名</span><input v-model="domainFilter" type="text" placeholder="example.com" /></label>
          <label><span>分类</span><input v-model="categoryFilter" type="text" placeholder="开发工具" /></label>
          <label><span>开始</span><input v-model="startTime" type="time" /></label>
          <label><span>结束</span><input v-model="endTime" type="time" /></label>
          <button class="text-button" type="button" :disabled="searching" @click="submitSearch"><Search :size="14" />应用筛选</button>
          <button class="text-button is-danger" type="button" :disabled="actionBusy === 'bulk-delete'" @click="removeSelection"><Trash2 :size="14" />删除范围</button>
        </div>

        <p v-if="searchError" class="inline-error">{{ searchError }}</p>
        <div v-if="shownActivities.length" class="activity-timeline">
          <article v-for="(item, index) in shownActivities" :key="item.activity.eventId" class="timeline-row">
            <div class="timeline-row__time">
              <strong>{{ new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" }).format(new Date(item.activity.ts)) }}</strong>
              <span>{{ index === 0 && selectedDate === localDate() && isFreshActivity(item.activity, props.nowMs) ? "进行中" : formatDuration(item.durationMs) }}</span>
            </div>
            <span class="timeline-row__dot" :class="`is-${item.activity.presence}`" />
            <div class="timeline-row__content">
              <div>
                <span class="app-symbol">{{ item.activity.app.name.slice(0, 1).toUpperCase() }}</span>
                <div>
                  <strong>{{ activityHeadline(item.activity) }}</strong>
                  <p class="timeline-row__status">{{ deriveStatusFromActivity(item.activity) }}</p>
                  <p>{{ activitySubline(item.activity) }}</p>
                </div>
              </div>
              <span>{{ item.activity.app.name }} · {{ item.categoryLabel }}</span>
              <div class="timeline-row__commands">
                <button class="icon-button is-danger" type="button" :disabled="actionBusy === item.activity.eventId" aria-label="删除活动" title="删除活动" @click="removeActivity(item.activity)"><Trash2 :size="14" /></button>
              </div>
              <a v-if="activityUrl(item.activity)" class="timeline-url" :href="activityUrl(item.activity) || undefined" target="_blank" rel="noreferrer">{{ activityUrl(item.activity) }}</a>
              <button v-if="item.screenshot" type="button" class="timeline-screenshot" @click="selectedScreenshot = item.screenshot ?? null">
                <img :src="item.screenshot.thumbnailUrl" alt="" loading="lazy" />
                <span><Image :size="14" />查看截图<i v-if="item.screenshot.ocrStatus === 'complete'"><ScanText :size="13" />OCR</i></span>
              </button>
            </div>
          </article>
        </div>
        <div v-else class="empty-inline">{{ searchQuery.trim() ? "没有匹配的活动" : "暂无活动记录" }}</div>
        <footer v-if="timeline && timeline.total > pageSize" class="timeline-pagination"><button class="icon-button" type="button" :disabled="pageOffset === 0" title="上一页" @click="changePage(pageOffset - pageSize)"><ChevronLeft :size="16" /></button><span>{{ pageOffset + 1 }}–{{ Math.min(pageOffset + pageSize, timeline.total) }} / {{ timeline.total }}</span><button class="icon-button" type="button" :disabled="pageOffset + pageSize >= timeline.total" title="下一页" @click="changePage(pageOffset + pageSize)"><ChevronRight :size="16" /></button></footer>
      </section>
    </template>
  </div>

  <div v-if="selectedScreenshot" class="media-viewer" role="dialog" aria-modal="true" aria-label="活动截图" @click.self="closeScreenshot">
    <section>
      <header><div><span class="section-kicker">Screenshot</span><strong>{{ formatDateTime(selectedScreenshot.capturedAt) }}</strong></div><div class="media-viewer__actions"><button class="icon-button" type="button" :disabled="actionBusy === selectedScreenshot.id" aria-label="重新识别 OCR" title="重新识别 OCR" @click="retryOcr(selectedScreenshot)"><RefreshCw :size="16" /></button><button class="icon-button is-danger" type="button" :disabled="actionBusy === selectedScreenshot.id" aria-label="删除截图" title="删除截图" @click="removeScreenshot(selectedScreenshot)"><Trash2 :size="16" /></button><button class="icon-button" type="button" aria-label="关闭截图" title="关闭" @click="closeScreenshot"><X :size="18" /></button></div></header>
      <img :src="selectedScreenshot.contentUrl" alt="活动截图" />
      <div class="media-viewer__ocr"><span><ScanText :size="15" />OCR · {{ selectedScreenshot.ocrStatus }}</span><p v-if="selectedScreenshot.ocrText">{{ selectedScreenshot.ocrText }}</p><p v-else-if="selectedScreenshot.ocrError">{{ selectedScreenshot.ocrError }}</p><p v-else>暂无可提取文字</p></div>
    </section>
  </div>
</template>
