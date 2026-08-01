<script setup lang="ts">
import { AlertTriangle, HardDrive, Image, MonitorCog, RefreshCw, ScanText, ShieldCheck, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { cleanupMedia, deleteScreenshot, fetchAgentDiagnostics, fetchMediaStatus, fetchRemoteMirrorStatus, fetchScreenshots, retryRemoteMirrors, retryScreenshotOcr } from "../api";
import { formatDateTime } from "../lib/activity";
import type { AgentDiagnostics, MediaStatus, RemoteMirrorStatus, ScreenshotRecord } from "../types";

const status = ref<MediaStatus | null>(null);
const screenshots = ref<ScreenshotRecord[]>([]);
const agents = ref<AgentDiagnostics[]>([]);
const remote = ref<RemoteMirrorStatus | null>(null);
const loading = ref(true);
const busyId = ref<string | null>(null);
const error = ref<string | null>(null);
const notice = ref<string | null>(null);
const capacityPercent = computed(() => status.value ? Math.min(100, status.value.totalBytes / status.value.totalLimitBytes * 100) : 0);

function bytes(value: number) {
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 * 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
  return `${(value / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function permissionLabel(value: string) {
  return value === "granted" ? "已授权" : value === "not_required" ? "无需授权" : "缺少权限";
}

async function load() {
  loading.value = true;
  try {
    const [nextStatus, nextScreenshots, diagnostics, nextRemote] = await Promise.all([
      fetchMediaStatus(), fetchScreenshots(120), fetchAgentDiagnostics(), fetchRemoteMirrorStatus()
    ]);
    status.value = nextStatus;
    screenshots.value = nextScreenshots.screenshots;
    agents.value = diagnostics.agents;
    remote.value = nextRemote;
    error.value = null;
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { loading.value = false; }
}

async function runCleanup() {
  if (!window.confirm("立即执行留存和容量清理？")) return;
  busyId.value = "cleanup";
  try {
    const result = await cleanupMedia();
    notice.value = `已删除 ${result.deletedScreenshots} 张，释放 ${bytes(result.freedBytes)}`;
    await load();
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { busyId.value = null; }
}

async function removeScreenshot(item: ScreenshotRecord) {
  if (!window.confirm(`删除 ${formatDateTime(item.capturedAt)} 的截图？活动记录会保留。`)) return;
  busyId.value = item.id;
  try { await deleteScreenshot(item.id); screenshots.value = screenshots.value.filter((value) => value.id !== item.id); status.value = await fetchMediaStatus(); }
  catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { busyId.value = null; }
}

async function retryOcr(item: ScreenshotRecord) {
  busyId.value = item.id;
  try {
    const updated = await retryScreenshotOcr(item.id);
    screenshots.value = screenshots.value.map((value) => value.id === item.id ? updated : value);
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { busyId.value = null; }
}

async function retryRemote() {
  busyId.value = "remote";
  try { const result = await retryRemoteMirrors(); notice.value = `已重新排队 ${result.queued} 个远程镜像`; await load(); }
  catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { busyId.value = null; }
}

onMounted(load);
</script>

<template>
  <div class="page-stack">
    <header class="page-heading">
      <div><span class="section-kicker">Media & Privacy</span><h1>采集与媒体</h1><p>设备权限、隐私策略和截图存储状态</p></div>
      <div class="page-heading__tools">
        <button class="text-button" type="button" :disabled="loading" @click="load"><RefreshCw :size="15" />刷新</button>
        <button class="primary-button" type="button" :disabled="busyId === 'cleanup'" @click="runCleanup"><Trash2 :size="15" />执行清理</button>
      </div>
    </header>

    <p v-if="error" class="inline-error">{{ error }}</p>
    <p v-if="notice" class="inline-status">{{ notice }}</p>
    <div v-if="loading" class="state-view"><span class="spinner" /> 正在读取媒体状态</div>
    <template v-else>
      <section v-if="status" class="metric-grid metric-grid--device">
        <article class="metric-card is-primary"><span><HardDrive :size="16" />原图存储</span><strong>{{ bytes(status.totalBytes) }}</strong><small>{{ status.screenshotCount }} 张 · 上限 {{ bytes(status.totalLimitBytes) }}</small></article>
        <article class="metric-card"><span><Image :size="16" />缩略图</span><strong>{{ bytes(status.thumbnailBytes) }}</strong><small>列表不再加载原图</small></article>
        <article class="metric-card"><span><ScanText :size="16" />OCR 队列</span><strong>{{ status.pendingOcr }}</strong><small>{{ status.failedOcr }} 个失败任务</small></article>
        <article class="metric-card"><span><ShieldCheck :size="16" />留存策略</span><strong>{{ status.retentionDays }} 天</strong><small>容量 {{ capacityPercent.toFixed(1) }}%</small></article>
      </section>

      <section v-if="remote" class="remote-status-bar"><span><HardDrive :size="15" />远程镜像</span><strong>{{ remote.configured ? remote.provider?.toUpperCase() : "未配置" }}</strong><span v-if="remote.configured">完成 {{ remote.complete }} · 等待 {{ remote.pending }} · 失败 {{ remote.failed }}</span><small v-if="remote.lastError">{{ remote.lastError }}</small><button v-if="remote.configured && (remote.failed || remote.pending)" class="text-button" type="button" :disabled="busyId === 'remote'" @click="retryRemote"><RefreshCw :size="14" />重试</button></section>

      <section class="data-section">
        <header class="section-heading"><div><span class="section-kicker">Agents</span><h2>设备采集状态</h2></div><span>{{ agents.length }} 台</span></header>
        <div v-if="agents.length" class="diagnostics-list">
          <article v-for="agent in agents" :key="agent.deviceId" class="diagnostics-row">
            <span class="device-avatar"><MonitorCog :size="19" /></span>
            <div><strong>{{ agent.deviceId }}</strong><small>{{ agent.platform }} · {{ formatDateTime(agent.updatedAt) }}</small></div>
            <dl>
              <div><dt>辅助功能</dt><dd :class="{ 'is-warning': agent.accessibilityPermission === 'missing' }">{{ permissionLabel(agent.accessibilityPermission) }}</dd></div>
              <div><dt>屏幕录制</dt><dd :class="{ 'is-warning': agent.screenCapturePermission === 'missing' }">{{ permissionLabel(agent.screenCapturePermission) }}</dd></div>
              <div><dt>截图</dt><dd>{{ agent.screenshotEnabled ? `${agent.screenshotFormat} · ${agent.screenshotDisplay}` : "关闭" }}</dd></div>
              <div><dt>活动采集</dt><dd>{{ agent.recordingEnabled ? `运行中 · r${agent.controlRevision}` : `已暂停 · r${agent.controlRevision}` }}</dd></div>
              <div><dt>隐私 / 待发送</dt><dd>{{ agent.privacyRuleCount }} 条 / {{ agent.spoolPending }} 项</dd></div>
            </dl>
            <AlertTriangle v-if="agent.accessibilityPermission === 'missing' || (agent.screenshotEnabled && agent.screenCapturePermission === 'missing')" class="diagnostics-warning" :size="18" />
          </article>
        </div>
        <div v-else class="empty-inline">尚未收到 Agent 诊断信息</div>
      </section>

      <section class="media-library">
        <header class="section-heading"><div><span class="section-kicker">Screenshots</span><h2>最近截图</h2></div><span>{{ screenshots.length }} 张</span></header>
        <div v-if="screenshots.length" class="media-grid">
          <article v-for="item in screenshots" :key="item.id" class="media-item">
            <a :href="item.contentUrl" target="_blank" rel="noreferrer"><img :src="item.thumbnailUrl" alt="活动截图缩略图" loading="lazy" /></a>
            <div class="media-item__body">
              <strong>{{ item.deviceId }}</strong><time>{{ formatDateTime(item.capturedAt) }}</time>
              <span>{{ bytes(item.byteSize) }} · {{ item.mimeType.replace('image/', '').toUpperCase() }}<i v-if="item.duplicateOf">OCR 复用</i></span>
            </div>
            <div class="media-item__actions">
              <button class="icon-button" type="button" :disabled="busyId === item.id" aria-label="重新识别 OCR" title="重新识别 OCR" @click="retryOcr(item)"><ScanText :size="15" /></button>
              <button class="icon-button is-danger" type="button" :disabled="busyId === item.id" aria-label="删除截图" title="删除截图" @click="removeScreenshot(item)"><Trash2 :size="15" /></button>
            </div>
          </article>
        </div>
        <div v-else class="empty-inline">暂无截图</div>
      </section>
    </template>
  </div>
</template>
