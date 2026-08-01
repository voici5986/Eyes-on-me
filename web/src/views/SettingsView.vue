<script setup lang="ts">
import { ArrowDown, ArrowUp, Download, Plus, Save, Settings2, Trash2, Upload } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { fetchReviewSettings, importReviewSettings, saveReviewSettings } from "../api";
import type { CategoryRule, ReviewSettings, WorkScheduleSegment } from "../types";

const settings = ref<ReviewSettings | null>(null);
const loading = ref(true);
const saving = ref(false);
const error = ref<string | null>(null);
const notice = ref<string | null>(null);
const importInput = ref<HTMLInputElement | null>(null);
const weekdays = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
const blocks = [
  { id: "overview", label: "概览" },
  { id: "applications", label: "应用使用" },
  { id: "websites", label: "浏览站点" },
  { id: "screenshots", label: "截图线索" }
];
const orderedBlocks = computed(() => settings.value?.reportPreferences.blockOrder ?? []);

function newId(prefix: string) {
  return `${prefix}-${globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`}`;
}

async function load() {
  loading.value = true;
  try {
    settings.value = await fetchReviewSettings();
    error.value = null;
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { loading.value = false; }
}

function addRule() {
  settings.value?.categoryRules.push({ id: newId("category"), name: "新分类", color: "#4f7667", target: "app", pattern: "", priority: 100 });
}
function removeRule(id: string) {
  if (settings.value) settings.value.categoryRules = settings.value.categoryRules.filter((item) => item.id !== id);
}
function addSchedule() {
  settings.value?.workSchedule.push({ id: newId("schedule"), weekday: 1, startMinute: 540, endMinute: 1080 });
}
function removeSchedule(id: string) {
  if (settings.value) settings.value.workSchedule = settings.value.workSchedule.filter((item) => item.id !== id);
}
function minuteText(value: number) {
  return `${String(Math.floor(value / 60)).padStart(2, "0")}:${String(value % 60).padStart(2, "0")}`;
}
function setMinutes(segment: WorkScheduleSegment, field: "startMinute" | "endMinute", value: string) {
  const [hours, minutes] = value.split(":").map(Number);
  segment[field] = hours * 60 + minutes;
}
function toggleList(list: string[], id: string, enabled: boolean) {
  const index = list.indexOf(id);
  if (enabled && index < 0) list.push(id);
  if (!enabled && index >= 0) list.splice(index, 1);
}
function moveBlock(id: string, delta: number) {
  if (!settings.value) return;
  const order = settings.value.reportPreferences.blockOrder;
  const from = order.indexOf(id);
  const to = from + delta;
  if (from < 0 || to < 0 || to >= order.length) return;
  [order[from], order[to]] = [order[to], order[from]];
}
function blockLabel(id: string) { return blocks.find((item) => item.id === id)?.label ?? id; }

async function save() {
  if (!settings.value) return;
  saving.value = true;
  try {
    settings.value = await saveReviewSettings(settings.value);
    notice.value = "设置已保存，历史统计会按新规则重新计算";
    error.value = null;
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { saving.value = false; }
}

async function importBackup(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0];
  if (!file) return;
  try {
    settings.value = await importReviewSettings(file);
    notice.value = "设置备份已恢复";
    error.value = null;
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { if (importInput.value) importInput.value.value = ""; }
}

onMounted(load);
</script>

<template>
  <div class="page-stack">
    <header class="page-heading">
      <div><span class="section-kicker">Settings</span><h1>回顾设置</h1><p>分类、工作时段、日报结构与备份</p></div>
      <div class="page-heading__tools">
        <input ref="importInput" class="visually-hidden" type="file" accept="application/json,.json" @change="importBackup" />
        <button class="text-button" type="button" @click="importInput?.click()"><Upload :size="15" />恢复</button>
        <a class="text-button" href="/api/settings/review/export" download><Download :size="15" />备份</a>
        <button class="primary-button" type="button" :disabled="saving || !settings" @click="save"><Save :size="15" />{{ saving ? "保存中" : "保存" }}</button>
      </div>
    </header>

    <div v-if="loading" class="state-view"><span class="spinner" />正在加载设置</div>
    <p v-else-if="error" class="inline-error">{{ error }}</p>
    <p v-if="notice" class="inline-notice">{{ notice }}</p>

    <template v-if="settings">
      <section class="data-section settings-section">
        <header class="section-heading"><div><span class="section-kicker">Categories</span><h2>应用与域名分类</h2></div><button class="icon-button" type="button" title="添加分类规则" aria-label="添加分类规则" @click="addRule"><Plus :size="16" /></button></header>
        <div class="settings-table category-rule-table">
          <div class="settings-table__head"><span>颜色</span><span>分类</span><span>目标</span><span>匹配规则</span><span>优先级</span><span /> </div>
          <div v-for="rule in settings.categoryRules" :key="rule.id" class="settings-table__row">
            <input v-model="rule.color" class="color-input" type="color" :aria-label="`${rule.name} 颜色`" />
            <input v-model="rule.name" type="text" aria-label="分类名称" />
            <select v-model="rule.target" aria-label="分类目标"><option value="app">应用</option><option value="domain">域名</option></select>
            <input v-model="rule.pattern" type="text" placeholder="*github.com" aria-label="匹配规则" />
            <input v-model.number="rule.priority" type="number" min="-1000" max="1000" aria-label="优先级" />
            <button class="icon-button is-danger" type="button" title="删除规则" aria-label="删除规则" @click="removeRule(rule.id)"><Trash2 :size="14" /></button>
          </div>
          <div v-if="!settings.categoryRules.length" class="empty-inline">当前使用内置分类规则</div>
        </div>
      </section>

      <section class="data-section settings-section">
        <header class="section-heading"><div><span class="section-kicker">Working hours</span><h2>多段工作时段</h2></div><button class="icon-button" type="button" title="添加工作时段" aria-label="添加工作时段" @click="addSchedule"><Plus :size="16" /></button></header>
        <div class="schedule-grid">
          <div v-for="segment in settings.workSchedule" :key="segment.id" class="schedule-row">
            <select v-model.number="segment.weekday" aria-label="星期"><option v-for="(label, index) in weekdays" :key="label" :value="index + 1">{{ label }}</option></select>
            <input :value="minuteText(segment.startMinute)" type="time" aria-label="开始时间" @input="setMinutes(segment, 'startMinute', ($event.target as HTMLInputElement).value)" />
            <span>至</span>
            <input :value="minuteText(segment.endMinute)" type="time" aria-label="结束时间" @input="setMinutes(segment, 'endMinute', ($event.target as HTMLInputElement).value)" />
            <button class="icon-button is-danger" type="button" title="删除时段" aria-label="删除时段" @click="removeSchedule(segment.id)"><Trash2 :size="14" /></button>
          </div>
        </div>
      </section>

      <section class="data-section settings-section">
        <header class="section-heading"><div><span class="section-kicker">Reports</span><h2>日报区块</h2></div><Settings2 :size="18" /></header>
        <div class="report-block-list">
          <div v-for="(id, index) in orderedBlocks" :key="id" class="report-block-row">
            <strong>{{ blockLabel(id) }}</strong>
            <label class="check-control"><input type="checkbox" :checked="settings.reportPreferences.pinnedBlocks.includes(id)" @change="toggleList(settings.reportPreferences.pinnedBlocks, id, ($event.target as HTMLInputElement).checked)" />置顶</label>
            <label class="check-control"><input type="checkbox" :checked="settings.reportPreferences.hiddenBlocks.includes(id)" @change="toggleList(settings.reportPreferences.hiddenBlocks, id, ($event.target as HTMLInputElement).checked)" />隐藏</label>
            <button class="icon-button" type="button" :disabled="index === 0" title="上移" aria-label="上移" @click="moveBlock(id, -1)"><ArrowUp :size="14" /></button>
            <button class="icon-button" type="button" :disabled="index === orderedBlocks.length - 1" title="下移" aria-label="下移" @click="moveBlock(id, 1)"><ArrowDown :size="14" /></button>
          </div>
        </div>
        <div class="auto-export-row">
          <label class="toggle-control"><input v-model="settings.reportPreferences.autoExportEnabled" type="checkbox" /><span />自动导出</label>
          <input v-model="settings.reportPreferences.autoExportDirectory" type="text" :disabled="!settings.reportPreferences.autoExportEnabled" placeholder="/absolute/path/to/reports" aria-label="自动导出目录" />
        </div>
      </section>
    </template>
  </div>
</template>
