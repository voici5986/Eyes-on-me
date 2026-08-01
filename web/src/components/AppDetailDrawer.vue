<script setup lang="ts">
import { AppWindow, Clock3, X } from "@lucide/vue";
import { nextTick, onBeforeUnmount, ref, watch } from "vue";
import { formatDateTime, formatDurationLong, usageShare } from "../lib/activity";
import type { AppUsageBucket } from "../types";

const props = defineProps<{
  app: AppUsageBucket | null;
}>();

const emit = defineEmits<{
  close: [];
}>();
const dialog = ref<HTMLElement | null>(null);
const closeButton = ref<HTMLButtonElement | null>(null);
let previousFocus: HTMLElement | null = null;

function focusableElements() {
  return Array.from(
    dialog.value?.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
    ) ?? []
  );
}

function handleKeydown(event: KeyboardEvent) {
  if (!props.app) return;

  if (event.key === "Escape") {
    event.preventDefault();
    emit("close");
    return;
  }

  if (event.key !== "Tab") return;

  const focusable = focusableElements();
  const first = focusable[0];
  const last = focusable.at(-1);
  if (!first || !last) {
    event.preventDefault();
    return;
  }

  const activeElement = document.activeElement;
  const focusIsOutside = !dialog.value?.contains(activeElement);
  if (event.shiftKey && (activeElement === first || focusIsOutside)) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && (activeElement === last || focusIsOutside)) {
    event.preventDefault();
    first.focus();
  }
}

watch(
  () => props.app,
  async (app) => {
    document.body.classList.toggle("drawer-open", Boolean(app));
    if (app) {
      previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      await nextTick();
      closeButton.value?.focus();
    } else if (previousFocus?.isConnected) {
      previousFocus.focus();
      previousFocus = null;
    }
  },
  { immediate: true }
);

window.addEventListener("keydown", handleKeydown);
onBeforeUnmount(() => {
  window.removeEventListener("keydown", handleKeydown);
  document.body.classList.remove("drawer-open");
});
</script>

<template>
  <Teleport to="body">
    <div v-if="app" class="drawer-layer" role="presentation" @mousedown.self="emit('close')">
      <aside ref="dialog" class="app-drawer" role="dialog" aria-modal="true" :aria-label="`${app.label} 窗口明细`">
        <header class="app-drawer__header">
          <div class="app-drawer__identity">
            <span class="app-drawer__icon"><AppWindow :size="20" /></span>
            <div>
              <span class="section-kicker">应用明细</span>
              <h2>{{ app.label }}</h2>
              <p>{{ app.sublabel }}</p>
            </div>
          </div>
          <button ref="closeButton" type="button" class="icon-button" aria-label="关闭应用明细" title="关闭" @click="emit('close')">
            <X :size="19" />
          </button>
        </header>

        <div class="app-drawer__summary">
          <div>
            <span>总使用时长</span>
            <strong>{{ formatDurationLong(app.totalTrackedMs) }}</strong>
          </div>
          <div>
            <span>窗口 / Tab</span>
            <strong>{{ app.windows.length }}</strong>
          </div>
          <div>
            <span>进入应用</span>
            <strong>{{ app.sessions }} 次</strong>
          </div>
        </div>

        <div class="app-drawer__section-title">
          <h3>窗口使用明细</h3>
          <span>按累计时长排序</span>
        </div>

        <div class="window-breakdown">
          <article v-for="window in app.windows" :key="window.key" class="window-row">
            <div class="window-row__main">
              <strong>{{ window.label }}</strong>
              <span v-if="window.sublabel">{{ window.sublabel }}</span>
            </div>
            <div class="window-row__value">
              <strong>{{ formatDurationLong(window.totalTrackedMs) }}</strong>
              <span><Clock3 :size="13" /> {{ window.sessions }} 次 · {{ formatDateTime(window.lastSeen) }}</span>
            </div>
            <div class="window-row__bar">
              <span :style="{ width: `${usageShare(app.totalTrackedMs, window.totalTrackedMs)}%` }" />
            </div>
          </article>
          <div v-if="app.windows.length === 0" class="empty-inline">暂无窗口明细</div>
        </div>
      </aside>
    </div>
  </Teleport>
</template>
