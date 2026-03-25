<script setup lang="ts">
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
  title: "Treemap",
  subtitle: "",
  height: 540,
  totalLabel: "总量"
});

const emit = defineEmits<{
  select: [node: TreemapNodeInput];
}>();

const stageRef = ref<HTMLElement | null>(null);
const stageWidth = ref(0);
const activePath = ref<string | null>(null);

let resizeObserver: ResizeObserver | null = null;

const layout = computed(() =>
  buildTreemapLayout(props.items, {
    width: Math.max(stageWidth.value, 0),
    height: props.height
  })
);
const valueFormatter = computed(() => props.valueFormatter ?? formatCompactValue);

const activeNode = computed(
  () => layout.value.nodes.find((node) => node.path === activePath.value) ?? firstInteractiveNode(layout.value.nodes)
);

const leafNodes = computed(() => layout.value.nodes.filter((node) => !node.hasChildren));

onMounted(() => {
  if (!stageRef.value) {
    return;
  }

  const updateWidth = () => {
    stageWidth.value = stageRef.value?.clientWidth ?? 0;
  };

  updateWidth();
  resizeObserver = new ResizeObserver(updateWidth);
  resizeObserver.observe(stageRef.value);
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  resizeObserver = null;
});

function nodeStyle(node: TreemapLayoutNode) {
  return {
    left: `${node.x}px`,
    top: `${node.y}px`,
    width: `${Math.max(node.width, 0)}px`,
    height: `${Math.max(node.height, 0)}px`,
    zIndex: `${node.depth + 1}`,
    background: node.hasChildren ? groupBackground(node.depth) : leafBackground(node, layout.value.metricMin, layout.value.metricMax),
    borderColor: node.hasChildren ? groupBorder(node.depth) : leafBorder(node, layout.value.metricMin, layout.value.metricMax)
  };
}

function groupBackground(depth: number): string {
  const alpha = Math.max(0.12, 0.2 - depth * 0.03);
  return `rgba(255, 255, 255, ${alpha})`;
}

function groupBorder(depth: number): string {
  const alpha = Math.max(0.12, 0.22 - depth * 0.02);
  return `rgba(255, 255, 255, ${alpha})`;
}

function leafBackground(node: TreemapLayoutNode, min: number, max: number): string {
  if (node.accent) {
    return `linear-gradient(180deg, ${withAlpha(node.accent, 0.95)} 0%, ${withAlpha(node.accent, 0.72)} 100%)`;
  }

  const metric = node.colorValue ?? node.value;
  const normalized = normalizeMetric(metric, min, max);
  const from = mixColor("#10202a", "#f0b36d", normalized * 0.78);
  const to = mixColor("#1b3240", "#ff6b57", normalized);

  return `linear-gradient(160deg, ${from} 0%, ${to} 100%)`;
}

function leafBorder(node: TreemapLayoutNode, min: number, max: number): string {
  if (node.accent) {
    return withAlpha(node.accent, 0.85);
  }

  const metric = node.colorValue ?? node.value;
  const normalized = normalizeMetric(metric, min, max);
  return mixColor("rgba(255,255,255,0.18)", "#ffd7a6", normalized);
}

function normalizeMetric(value: number, min: number, max: number): number {
  if (min === max) {
    return 0.5;
  }

  if (min < 0 && max > 0) {
    if (value < 0) {
      return 0.5 * (value - min) / (0 - min);
    }

    return 0.5 + 0.5 * value / max;
  }

  return (value - min) / (max - min);
}

function mixColor(from: string, to: string, ratio: number): string {
  if (from.startsWith("rgba")) {
    return from;
  }

  const safe = clamp(ratio, 0, 1);
  const fromValue = Number.parseInt(from.slice(1), 16);
  const toValue = Number.parseInt(to.slice(1), 16);

  const r = Math.round(((fromValue >> 16) & 255) + ((((toValue >> 16) & 255) - ((fromValue >> 16) & 255)) * safe));
  const g = Math.round(((fromValue >> 8) & 255) + ((((toValue >> 8) & 255) - ((fromValue >> 8) & 255)) * safe));
  const b = Math.round((fromValue & 255) + (((toValue & 255) - (fromValue & 255)) * safe));

  return `rgb(${r}, ${g}, ${b})`;
}

function withAlpha(color: string, alpha: number): string {
  const hex = color.replace("#", "");
  const safe = hex.length === 3 ? hex.split("").map((channel) => channel + channel).join("") : hex;

  if (safe.length !== 6) {
    return color;
  }

  const value = Number.parseInt(safe, 16);
  const r = (value >> 16) & 255;
  const g = (value >> 8) & 255;
  const b = value & 255;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function labelVisible(node: TreemapLayoutNode): boolean {
  return node.width >= 78 && node.height >= 46 && node.area >= 3200;
}

function metaVisible(node: TreemapLayoutNode): boolean {
  return node.width >= 110 && node.height >= 82 && node.area >= 7000;
}

function noteVisible(node: TreemapLayoutNode): boolean {
  return node.width >= 150 && node.height >= 118 && node.area >= 13000;
}

function valueVisible(node: TreemapLayoutNode): boolean {
  return node.width >= 96 && node.height >= 60 && node.area >= 4200;
}

function percentageText(node: TreemapLayoutNode): string {
  return `${(node.share * 100).toFixed(node.share >= 0.1 ? 1 : 2)}%`;
}

function selectNode(node: TreemapLayoutNode) {
  activePath.value = node.path;
  emit("select", node.source);
}

function firstInteractiveNode(nodes: TreemapLayoutNode[]): TreemapLayoutNode | null {
  return nodes.find((node) => !node.hasChildren) ?? nodes[0] ?? null;
}

function formatCompactValue(value: number): string {
  return new Intl.NumberFormat("zh-CN", {
    notation: value >= 10000 ? "compact" : "standard",
    maximumFractionDigits: value >= 10000 ? 1 : 0
  }).format(value);
}
</script>

<template>
  <section class="large-treemap">
    <header class="large-treemap__header">
      <div>
        <p class="large-treemap__eyebrow">Treemap</p>
        <h3>{{ title }}</h3>
        <p v-if="subtitle" class="large-treemap__subtitle">{{ subtitle }}</p>
      </div>
      <div class="large-treemap__summary">
        <span>{{ totalLabel }}</span>
        <strong>{{ valueFormatter(layout.total) }}</strong>
      </div>
    </header>

    <div ref="stageRef" class="large-treemap__stage" :style="{ height: `${height}px` }">
      <div
        v-for="node in layout.nodes"
        :key="node.path"
        class="large-treemap__node"
        :class="{
          'is-group': node.hasChildren,
          'is-leaf': !node.hasChildren,
          'is-active': activeNode?.path === node.path
        }"
        :style="nodeStyle(node)"
        @mouseenter="activePath = node.path"
        @focusin="activePath = node.path"
        @click="selectNode(node)"
      >
        <div class="large-treemap__node-shell">
          <span v-if="labelVisible(node)" class="large-treemap__label">
            {{ node.label }}
          </span>
          <span v-if="metaVisible(node) && node.meta" class="large-treemap__meta">
            {{ node.meta }}
          </span>
          <span v-if="valueVisible(node)" class="large-treemap__value">
            {{ valueFormatter(node.value) }}
          </span>
          <span v-if="valueVisible(node) && !node.hasChildren" class="large-treemap__share">
            {{ percentageText(node) }}
          </span>
          <span v-if="noteVisible(node) && node.note" class="large-treemap__note">
            {{ node.note }}
          </span>
        </div>
      </div>
    </div>

    <footer class="large-treemap__footer">
      <div v-if="activeNode" class="large-treemap__detail">
        <span class="large-treemap__detail-label">当前焦点</span>
        <strong>{{ activeNode.label }}</strong>
        <span>{{ valueFormatter(activeNode.value) }} · {{ percentageText(activeNode) }}</span>
        <span v-if="activeNode.meta">{{ activeNode.meta }}</span>
        <span v-if="activeNode.note">{{ activeNode.note }}</span>
      </div>

      <div v-else class="large-treemap__detail large-treemap__detail--idle">
        <span class="large-treemap__detail-label">当前焦点</span>
        <strong>暂无节点</strong>
      </div>

      <div class="large-treemap__legend">
        <span>低</span>
        <div class="large-treemap__legend-bar" />
        <span>高</span>
      </div>
    </footer>
  </section>
</template>

<style scoped>
.large-treemap {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.large-treemap__header,
.large-treemap__footer {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 16px;
}

.large-treemap__eyebrow {
  margin: 0 0 8px;
  color: #f0b36d;
  text-transform: uppercase;
  letter-spacing: 0.12em;
  font-size: 11px;
}

.large-treemap__header h3 {
  margin: 0;
  font-size: clamp(24px, 3vw, 32px);
  line-height: 1;
}

.large-treemap__subtitle {
  max-width: 620px;
  margin: 8px 0 0;
  color: rgba(246, 244, 238, 0.64);
  line-height: 1.5;
}

.large-treemap__summary,
.large-treemap__detail,
.large-treemap__legend {
  padding: 12px 14px;
  border-radius: 18px;
  border: 1px solid rgba(255, 255, 255, 0.08);
  background: rgba(255, 255, 255, 0.03);
}

.large-treemap__summary {
  display: flex;
  min-width: 130px;
  flex-direction: column;
  align-items: flex-end;
  gap: 6px;
}

.large-treemap__summary span,
.large-treemap__detail-label {
  color: rgba(246, 244, 238, 0.56);
  font-size: 12px;
  text-transform: uppercase;
  letter-spacing: 0.08em;
}

.large-treemap__summary strong {
  font-size: 24px;
  line-height: 1;
}

.large-treemap__stage {
  position: relative;
  overflow: hidden;
  border-radius: 28px;
  border: 1px solid rgba(255, 255, 255, 0.08);
  background:
    radial-gradient(circle at top left, rgba(240, 179, 109, 0.12), transparent 24%),
    linear-gradient(180deg, rgba(14, 20, 27, 0.92) 0%, rgba(8, 13, 18, 0.96) 100%);
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.05);
}

.large-treemap__node {
  position: absolute;
  padding: 4px;
  border: 1px solid transparent;
  border-radius: 18px;
  transition:
    transform 160ms ease,
    box-shadow 160ms ease,
    border-color 160ms ease;
  cursor: pointer;
}

.large-treemap__node:hover,
.large-treemap__node.is-active {
  transform: translateY(-1px);
  box-shadow: 0 12px 26px rgba(0, 0, 0, 0.22);
}

.large-treemap__node-shell {
  display: flex;
  height: 100%;
  min-width: 0;
  min-height: 0;
  flex-direction: column;
  justify-content: flex-start;
  gap: 4px;
  overflow: hidden;
  border-radius: 14px;
  padding: 10px;
}

.large-treemap__node.is-group .large-treemap__node-shell {
  justify-content: flex-start;
  padding: 8px 10px;
  background: linear-gradient(180deg, rgba(255, 255, 255, 0.04) 0%, rgba(255, 255, 255, 0.02) 100%);
}

.large-treemap__node.is-leaf .large-treemap__node-shell {
  justify-content: flex-end;
}

.large-treemap__label {
  display: block;
  overflow: hidden;
  font-size: 15px;
  font-weight: 700;
  line-height: 1.1;
  text-overflow: ellipsis;
}

.large-treemap__meta,
.large-treemap__note,
.large-treemap__share {
  color: rgba(246, 244, 238, 0.76);
  font-size: 12px;
  line-height: 1.35;
}

.large-treemap__note {
  display: -webkit-box;
  overflow: hidden;
  -webkit-line-clamp: 3;
  -webkit-box-orient: vertical;
}

.large-treemap__value {
  font-size: clamp(16px, 1.9vw, 22px);
  font-weight: 700;
  line-height: 1;
}

.large-treemap__detail {
  display: flex;
  min-height: 88px;
  flex: 1;
  flex-direction: column;
  justify-content: center;
  gap: 5px;
}

.large-treemap__detail strong {
  font-size: 18px;
}

.large-treemap__detail span:not(.large-treemap__detail-label) {
  color: rgba(246, 244, 238, 0.72);
}

.large-treemap__detail--idle {
  justify-content: center;
}

.large-treemap__legend {
  display: flex;
  min-width: 210px;
  align-items: center;
  gap: 10px;
}

.large-treemap__legend span {
  color: rgba(246, 244, 238, 0.62);
  font-size: 12px;
}

.large-treemap__legend-bar {
  height: 10px;
  flex: 1;
  border-radius: 999px;
  background: linear-gradient(90deg, rgb(20, 40, 52) 0%, rgb(219, 135, 95) 56%, rgb(255, 107, 87) 100%);
}

@media (max-width: 820px) {
  .large-treemap__header,
  .large-treemap__footer {
    flex-direction: column;
    align-items: stretch;
  }

  .large-treemap__summary {
    align-items: flex-start;
  }

  .large-treemap__legend {
    min-width: 0;
  }
}
</style>
