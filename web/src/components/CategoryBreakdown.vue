<script setup lang="ts">
import { computed } from "vue";
import { formatDurationLong, usageShare } from "../lib/activity";
import type { CategoryUsageBucket } from "../types";

const props = defineProps<{
  items: CategoryUsageBucket[];
  total: number;
}>();

const colors: Record<string, string> = {
  development: "#17795e",
  browser: "#3d6f91",
  communication: "#d3604c",
  office: "#b8871b",
  creative: "#7866a9",
  media: "#a05b7a",
  system: "#6d7470",
  other: "#9aa19d"
};
const visibleItems = computed(() => props.items.filter((item) => item.totalTrackedMs > 0));

function categoryColor(key: string): string {
  return colors[key] ?? colors.other;
}
</script>

<template>
  <div class="category-breakdown">
    <div class="category-breakdown__bar" aria-hidden="true">
      <span
        v-for="item in visibleItems"
        :key="item.key"
        :style="{
          width: `${usageShare(total, item.totalTrackedMs)}%`,
          backgroundColor: categoryColor(item.key)
        }"
      />
    </div>
    <div class="category-breakdown__list">
      <div v-for="item in visibleItems" :key="item.key" class="category-breakdown__row">
        <span class="category-breakdown__name">
          <i :style="{ backgroundColor: categoryColor(item.key) }" />
          {{ item.label }}
        </span>
        <span>{{ formatDurationLong(item.totalTrackedMs) }}</span>
        <strong>{{ usageShare(total, item.totalTrackedMs).toFixed(0) }}%</strong>
      </div>
    </div>
  </div>
</template>

