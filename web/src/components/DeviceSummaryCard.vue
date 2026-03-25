<script setup lang="ts">
import { RouterLink } from "vue-router";

interface CardAction {
  label: string;
  to: string;
}

defineProps<{
  title: string;
  headline: string;
  metaLine: string;
  summaryLine: string;
  url?: string | null;
  topBadge?: string;
  footerMeta?: string[];
  actions?: CardAction[];
}>();
</script>

<template>
  <li class="device-card">
    <div class="card-top">
      <div class="card-copy">
        <strong>{{ title }}</strong>
        <p>{{ headline }}</p>
      </div>
      <span v-if="topBadge" class="device-card__top-badge">{{ topBadge }}</span>
    </div>

    <span class="inline-meta">{{ metaLine }}</span>
    <p class="summary-line">{{ summaryLine }}</p>
    <code v-if="url" class="url">{{ url }}</code>

    <div class="card-footer">
      <div v-if="footerMeta?.length" class="card-footer-meta">
        <span
          v-for="item in footerMeta"
          :key="item"
          class="inline-meta card-update"
        >
          {{ item }}
        </span>
      </div>
      <span v-else />

      <div v-if="actions?.length" class="action-row card-actions">
        <RouterLink
          v-for="action in actions"
          :key="action.to + action.label"
          class="button-link"
          :to="action.to"
        >
          {{ action.label }}
        </RouterLink>
      </div>
    </div>
  </li>
</template>
