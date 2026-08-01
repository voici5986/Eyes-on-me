<script setup lang="ts">
import { computed } from "vue";
import { formatDurationLong } from "../lib/activity";
import type { DailyUsageBucket } from "../types";

const props = defineProps<{
  items: DailyUsageBucket[];
}>();

const maxDuration = computed(() => Math.max(1, ...props.items.map((item) => item.totalTrackedMs)));

function barHeight(value: number): number {
  return Math.max(2, (value / maxDuration.value) * 100);
}

function dateLabel(value: string): string {
  const parts = value.split("-");
  return parts.length === 3 ? `${parts[1]}/${parts[2]}` : value;
}
</script>

<template>
  <div class="daily-trend" role="img" aria-label="逐日活动趋势">
    <div v-for="item in items" :key="item.date" class="daily-trend__column">
      <div class="daily-trend__track" :title="`${item.date} · ${formatDurationLong(item.totalTrackedMs)}`">
        <span :style="{ height: `${barHeight(item.totalTrackedMs)}%` }" />
      </div>
      <span>{{ dateLabel(item.date) }}</span>
    </div>
  </div>
</template>

