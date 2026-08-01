<script setup lang="ts">
import { ChevronRight } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { buildTreemapLayout, type TreemapLayoutNode, type TreemapNodeInput } from "./treemap";

const props = withDefaults(defineProps<{
  items: TreemapNodeInput[];
  title?: string;
  subtitle?: string;
  height?: number;
  totalLabel?: string;
  valueFormatter?: (value: number) => string;
}>(), {
  title: "应用版图",
  subtitle: "",
  height: 520,
  totalLabel: "总量"
});

const emit = defineEmits<{ select: [node: TreemapNodeInput] }>();
const stageRef = ref<HTMLElement | null>(null);
const stageWidth = ref(0);
const activePath = ref<string | null>(null);
let resizeObserver: ResizeObserver | null = null;

const layout = computed(() => buildTreemapLayout(props.items, { width: stageWidth.value, height: props.height }));
const activeNode = computed(() => layout.value.nodes.find((node) => node.path === activePath.value) ?? layout.value.nodes[0] ?? null);
const valueFormatter = computed(() => props.valueFormatter ?? ((value: number) => new Intl.NumberFormat("zh-CN").format(value)));

onMounted(() => {
  if (!stageRef.value) return;
  const updateWidth = () => { stageWidth.value = stageRef.value?.clientWidth ?? 0; };
  updateWidth();
  resizeObserver = new ResizeObserver(updateWidth);
  resizeObserver.observe(stageRef.value);
});
onBeforeUnmount(() => resizeObserver?.disconnect());

function nodeStyle(node: TreemapLayoutNode) {
  const strength = normalizeMetric(node.colorValue ?? node.value, layout.value.metricMin, layout.value.metricMax);
  const accent = node.accent ?? "#17795e";
  return {
    left: `${node.x}px`,
    top: `${node.y}px`,
    width: `${Math.max(node.width, 0)}px`,
    height: `${Math.max(node.height, 0)}px`,
    zIndex: `${node.depth + 1}`,
    backgroundColor: node.hasChildren ? "#f1f3ef" : withAlpha(accent, 0.18 + strength * 0.5),
    borderColor: node.hasChildren ? "#dfe3dd" : withAlpha(accent, 0.34 + strength * 0.42)
  };
}

function normalizeMetric(value: number, min: number, max: number): number {
  return min === max ? 0.5 : Math.max(0, Math.min(1, (value - min) / (max - min)));
}

function withAlpha(color: string, alpha: number): string {
  const hex = color.replace("#", "");
  const safe = hex.length === 3 ? hex.split("").map((channel) => channel + channel).join("") : hex;
  if (safe.length !== 6) return color;
  const value = Number.parseInt(safe, 16);
  return `rgba(${(value >> 16) & 255}, ${(value >> 8) & 255}, ${value & 255}, ${alpha})`;
}

function isLabelVisible(node: TreemapLayoutNode) { return node.width >= 72 && node.height >= 42 && node.area >= 2800; }
function isMetaVisible(node: TreemapLayoutNode) { return node.width >= 110 && node.height >= 78 && node.area >= 6500; }
function isValueVisible(node: TreemapLayoutNode) { return node.width >= 92 && node.height >= 58 && node.area >= 4000; }
function percentage(node: TreemapLayoutNode) { return `${(node.share * 100).toFixed(node.share >= 0.1 ? 1 : 2)}%`; }

function selectNode(node: TreemapLayoutNode) {
  activePath.value = node.path;
  emit("select", node.source);
}
</script>

<template>
  <div class="large-treemap">
    <header class="large-treemap__header">
      <div>
        <span class="section-kicker">Treemap</span>
        <h3>{{ title }}</h3>
        <p v-if="subtitle">{{ subtitle }}</p>
      </div>
      <div class="large-treemap__total"><span>{{ totalLabel }}</span><strong>{{ valueFormatter(layout.total) }}</strong></div>
    </header>

    <div ref="stageRef" class="large-treemap__stage" :style="{ height: `${height}px` }">
      <button
        v-for="node in layout.nodes"
        :key="node.path"
        type="button"
        class="large-treemap__node"
        :class="{ 'is-active': activeNode?.path === node.path, 'is-group': node.hasChildren }"
        :style="nodeStyle(node)"
        :aria-label="`${node.label} ${valueFormatter(node.value)}`"
        @mouseenter="activePath = node.path"
        @focus="activePath = node.path"
        @click="selectNode(node)"
      >
        <span v-if="isLabelVisible(node)" class="large-treemap__label">{{ node.label }}</span>
        <span v-if="isMetaVisible(node) && node.meta" class="large-treemap__meta">{{ node.meta }}</span>
        <strong v-if="isValueVisible(node)">{{ valueFormatter(node.value) }}</strong>
        <span v-if="isValueVisible(node) && !node.hasChildren" class="large-treemap__share">{{ percentage(node) }}</span>
      </button>
    </div>

    <footer v-if="activeNode" class="large-treemap__footer">
      <div><span>当前选择</span><strong>{{ activeNode.label }}</strong><small>{{ activeNode.meta }}</small></div>
      <div><strong>{{ valueFormatter(activeNode.value) }}</strong><span>{{ percentage(activeNode) }}</span><ChevronRight :size="17" /></div>
    </footer>
  </div>
</template>

<style scoped>
.large-treemap { display: grid; gap: 16px; }
.large-treemap__header, .large-treemap__footer { display: flex; align-items: flex-end; justify-content: space-between; gap: 20px; }
.large-treemap__header h3 { margin: 5px 0 0; font-size: 20px; line-height: 1.2; }
.large-treemap__header p { margin: 6px 0 0; color: var(--text-muted); font-size: 13px; }
.large-treemap__total { display: grid; justify-items: end; gap: 3px; }
.large-treemap__total span, .large-treemap__footer span, .large-treemap__footer small { color: var(--text-muted); font-size: 12px; }
.large-treemap__total strong { font-size: 20px; }
.large-treemap__stage { position: relative; overflow: hidden; border: 1px solid var(--line); border-radius: 8px; background: #eef1ed; }
.large-treemap__node { position: absolute; display: flex; min-width: 0; min-height: 0; flex-direction: column; justify-content: flex-end; gap: 4px; overflow: hidden; padding: 11px; border: 3px solid; border-radius: 7px; color: var(--text); text-align: left; cursor: pointer; transition: border-color 150ms var(--ease-out), box-shadow 150ms var(--ease-out), transform 150ms var(--ease-out); }
.large-treemap__node.is-group { justify-content: flex-start; }
.large-treemap__node:focus-visible { outline: 2px solid var(--accent); outline-offset: -3px; }
.large-treemap__label { overflow: hidden; font-size: 14px; font-weight: 700; line-height: 1.2; text-overflow: ellipsis; white-space: nowrap; }
.large-treemap__meta, .large-treemap__share { overflow: hidden; color: rgba(20, 28, 24, 0.68); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.large-treemap__node strong { font-size: 17px; }
.large-treemap__footer { min-height: 56px; padding: 10px 12px; border-top: 1px solid var(--line); }
.large-treemap__footer > div { display: flex; align-items: center; gap: 10px; min-width: 0; }
.large-treemap__footer > div:first-child { flex-direction: column; align-items: flex-start; gap: 2px; }
.large-treemap__footer strong, .large-treemap__footer small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
@media (hover: hover) and (pointer: fine) { .large-treemap__node:hover { border-color: var(--accent); box-shadow: 0 5px 15px rgba(24, 36, 30, 0.12); transform: translateY(-1px); } }
.large-treemap__node:active { transform: scale(0.985); }
@media (max-width: 720px) { .large-treemap__header, .large-treemap__footer { align-items: flex-start; flex-direction: column; } .large-treemap__total { justify-items: start; } }
@media (prefers-reduced-motion: reduce) { .large-treemap__node { transition: border-color 120ms ease; } }
</style>
