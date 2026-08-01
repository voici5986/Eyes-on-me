<script setup lang="ts">
import { Brain, Database, RefreshCw, Search } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { reindexMemory, searchMemory } from "../api";
import type { MemorySearchResponse } from "../types";

const query = ref("");
const result = ref<MemorySearchResponse | null>(null);
const loading = ref(false);
const indexing = ref(false);
const status = ref<string | null>(null);
const error = ref<string | null>(null);
const modeLabel = computed(() => ({ semantic: "语义检索", full_text: "全文检索", recent: "最近记录" }[result.value?.mode ?? ""] ?? result.value?.mode));

async function search() {
  loading.value = true;
  try { result.value = await searchMemory(query.value); error.value = null; }
  catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { loading.value = false; }
}

async function reindex() {
  indexing.value = true;
  try {
    const response = await reindexMemory();
    status.value = `已整理 ${response.indexedEntries} 条，生成 ${response.embeddedEntries} 条向量`;
    await search();
  } catch (err) { error.value = err instanceof Error ? err.message : String(err); }
  finally { indexing.value = false; }
}

function sourceLabel(value: string) {
  return ({ activity: "活动", screenshot_ocr: "截图 OCR", daily_report: "日报" }[value] ?? value);
}

onMounted(search);
</script>

<template>
  <div class="page-stack">
    <header class="page-heading">
      <div><span class="section-kicker">Memory</span><h1>活动记忆</h1><p>跨活动、截图文字与日报检索历史上下文。</p></div>
      <button class="text-button" type="button" :disabled="indexing" @click="reindex"><RefreshCw :size="15" />{{ indexing ? "整理中" : "重建记忆" }}</button>
    </header>
    <section class="data-section memory-toolbar">
      <form class="activity-search" role="search" @submit.prevent="search"><Search :size="16" /><input v-model="query" type="search" placeholder="搜索项目、窗口、页面文字或日报" aria-label="搜索活动记忆" /><button class="activity-search__submit" :disabled="loading">搜索</button></form>
      <span v-if="result"><Brain :size="15" />{{ modeLabel }} · {{ result.entries.length }} 条</span>
    </section>
    <p v-if="status" class="inline-status">{{ status }}</p><p v-if="error" class="inline-error">{{ error }}</p>
    <div v-if="loading" class="state-view"><span class="spinner" /> 正在检索记忆</div>
    <section v-else-if="result?.entries.length" class="memory-list">
      <article v-for="entry in result.entries" :key="entry.id" class="memory-card">
        <header><span><Database :size="14" />{{ sourceLabel(entry.sourceType) }}</span><time>{{ entry.date }}</time></header>
        <h2>{{ entry.title }}</h2><p>{{ entry.content }}</p>
        <footer><div><span v-for="tag in entry.tags" :key="tag">{{ tag }}</span></div><strong v-if="entry.score != null">{{ (entry.score * 100).toFixed(1) }}%</strong></footer>
      </article>
    </section>
    <div v-else class="state-view"><Brain :size="24" /><strong>还没有可检索的记忆</strong><span>重建记忆后会聚合活动、OCR 和日报。</span></div>
  </div>
</template>
