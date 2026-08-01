<script setup lang="ts">
import { ANALYSIS_RANGE_OPTIONS } from "../lib/analysis-range";
import type { AnalysisRange } from "../types";

defineProps<{
  modelValue: AnalysisRange;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: AnalysisRange];
}>();
</script>

<template>
  <div class="range-control" role="tablist" aria-label="统计时间范围">
    <button
      v-for="option in ANALYSIS_RANGE_OPTIONS"
      :key="option.value"
      type="button"
      role="tab"
      class="range-control__option"
      :class="{ 'is-active': option.value === modelValue }"
      :aria-selected="option.value === modelValue"
      :title="option.description"
      @click="emit('update:modelValue', option.value)"
    >
      {{ option.label }}
    </button>
  </div>
</template>

