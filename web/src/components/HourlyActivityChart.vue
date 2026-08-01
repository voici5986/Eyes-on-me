<script setup lang="ts">
import { computed } from "vue";
import type { HourlyUsageBucket } from "../types";
import { formatDurationLong } from "../lib/activity";

const props = defineProps<{
  buckets: HourlyUsageBucket[];
}>();

const palette = ["#17795e", "#d3604c", "#3d6f91", "#b8871b", "#7866a9"];
const maxDuration = computed(() => Math.max(1, ...props.buckets.map((bucket) => bucket.totalTrackedMs)));
const topApps = computed(() => {
  const totals = new Map<string, { label: string; duration: number }>();
  for (const bucket of props.buckets) {
    for (const app of bucket.apps) {
      const current = totals.get(app.key) ?? { label: app.label, duration: 0 };
      current.duration += app.totalTrackedMs;
      totals.set(app.key, current);
    }
  }
  return [...totals.entries()]
    .sort((left, right) => right[1].duration - left[1].duration)
    .slice(0, palette.length)
    .map(([key, value], index) => ({ key, ...value, color: palette[index] }));
});
const colorByApp = computed(() => new Map(topApps.value.map((app) => [app.key, app.color])));

function fillHeight(bucket: HourlyUsageBucket): number {
  return Math.max(0, (bucket.totalTrackedMs / maxDuration.value) * 100);
}

function segmentHeight(bucket: HourlyUsageBucket, value: number): number {
  return bucket.totalTrackedMs > 0 ? (value / bucket.totalTrackedMs) * 100 : 0;
}
</script>

<template>
  <div class="hourly-chart">
    <div class="hourly-chart__legend" aria-label="主要应用图例">
      <span v-for="app in topApps" :key="app.key">
        <i :style="{ backgroundColor: app.color }" />
        {{ app.label }}
      </span>
    </div>

    <div class="hourly-chart__plot" role="img" aria-label="24 小时活动分布">
      <div v-for="bucket in buckets" :key="bucket.hour" class="hourly-chart__column">
        <div
          class="hourly-chart__track"
          :title="`${String(bucket.hour).padStart(2, '0')}:00 · ${formatDurationLong(bucket.totalTrackedMs)}`"
        >
          <div class="hourly-chart__fill" :style="{ height: `${fillHeight(bucket)}%` }">
            <span
              v-for="app in bucket.apps.filter((item) => colorByApp.has(item.key))"
              :key="app.key"
              :style="{
                height: `${segmentHeight(bucket, app.totalTrackedMs)}%`,
                backgroundColor: colorByApp.get(app.key)
              }"
            />
          </div>
        </div>
        <span class="hourly-chart__label">{{ bucket.hour % 3 === 0 ? String(bucket.hour).padStart(2, "0") : "" }}</span>
      </div>
    </div>
  </div>
</template>

