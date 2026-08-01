<script setup lang="ts">
import { Download, FileText, RefreshCw, Save } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { fetchDailyReport, fetchReports, generateDailyReport, saveDailyReport } from "../api";
import type { DailyReport } from "../types";

const date = ref(localDate());
const report = ref<Awaited<ReturnType<typeof fetchDailyReport>>>(null);
const content = ref("");
const useAi = ref(false);
const loading = ref(false);
const saving = ref(false);
const error = ref<string | null>(null);
const rangeStart = ref(`${date.value.slice(0, 8)}01`);
const rangeEnd = ref(date.value);
const history = ref<DailyReport[]>([]);
const hasChanges = computed(() => content.value !== (report.value?.content ?? ""));

function localDate() {
  const now = new Date();
  const offset = now.getTimezoneOffset() * 60_000;
  return new Date(now.getTime() - offset).toISOString().slice(0, 10);
}

async function load() {
  loading.value = true;
  try {
    report.value = await fetchDailyReport(date.value);
    content.value = report.value?.content ?? "";
    error.value = null;
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally { loading.value = false; }
}

async function loadHistory() {
  try { history.value = await fetchReports(rangeStart.value, rangeEnd.value); }
  catch (err) { error.value = err instanceof Error ? err.message : String(err); }
}

async function generate() {
  loading.value = true;
  try {
    report.value = await generateDailyReport(date.value, useAi.value);
    content.value = report.value.content;
    error.value = null;
    await loadHistory();
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally { loading.value = false; }
}

async function save() {
  saving.value = true;
  try {
    report.value = await saveDailyReport(date.value, content.value);
    content.value = report.value.content;
    error.value = null;
    await loadHistory();
  } catch (err) {
    error.value = err instanceof Error ? err.message : String(err);
  } finally { saving.value = false; }
}

function openReport(item: DailyReport) { date.value = item.date; void load(); }

onMounted(() => { void Promise.all([load(), loadHistory()]); });
</script>

<template>
  <div class="page-stack">
    <header class="page-heading">
      <div><span class="section-kicker">Reports</span><h1>活动日报</h1><p>由活动区间和截图 OCR 生成，可直接修订。</p></div>
      <div class="page-heading__tools">
        <input v-model="date" class="date-input" type="date" @change="load" />
        <label class="toggle-control"><input v-model="useAi" type="checkbox" /><span />AI 润色</label>
        <button class="text-button" type="button" :disabled="loading" @click="generate"><RefreshCw :size="15" />生成</button>
      </div>
    </header>
    <p v-if="error" class="inline-error">{{ error }}</p>
    <div v-if="loading" class="state-view"><span class="spinner" /> 正在整理日报</div>
    <section v-else class="report-layout">
      <article class="data-section report-editor">
        <header class="section-heading"><div><span class="section-kicker">Markdown</span><h2>日报正文</h2></div><span>{{ report?.generationMode ?? "尚未生成" }}</span></header>
        <textarea v-model="content" aria-label="日报正文" placeholder="选择日期后生成日报" />
        <footer>
          <span v-if="report?.fallbackReason">AI 未启用：{{ report.fallbackReason }}</span><span v-else>{{ hasChanges ? "有未保存修改" : "内容已保存" }}</span>
          <div><a v-if="report" class="text-button" :href="`/api/reports/${date}/export`" download><Download :size="15" />导出</a><button class="primary-button" type="button" :disabled="saving || !content.trim() || !hasChanges" @click="save"><Save :size="15" />{{ saving ? "保存中" : "保存" }}</button></div>
        </footer>
      </article>
      <aside class="data-section report-meta">
        <FileText :size="21" />
        <strong>{{ date }}</strong>
        <span>{{ report ? `更新于 ${new Date(report.updatedAt).toLocaleString()}` : "这一天还没有日报" }}</span>
        <dl v-if="report"><div><dt>生成方式</dt><dd>{{ report.generationMode }}</dd></div><div><dt>模型</dt><dd>{{ report.modelName ?? "未使用" }}</dd></div></dl>
        <div class="report-history-controls"><input v-model="rangeStart" type="date" @change="loadHistory" /><span>至</span><input v-model="rangeEnd" type="date" @change="loadHistory" /></div>
        <a class="text-button" :href="`/api/reports/export?start=${rangeStart}&end=${rangeEnd}`" download><Download :size="15" />导出范围</a>
        <div class="report-history"><button v-for="item in history" :key="item.date" type="button" :class="{ 'is-active': item.date === date }" @click="openReport(item)"><strong>{{ item.date }}</strong><span>{{ item.generationMode }}</span></button><span v-if="!history.length">范围内暂无日报</span></div>
      </aside>
    </section>
  </div>
</template>
