import type { TreemapNodeInput } from "../components/treemap";
import type { DeviceAnalysisResponse, UsageBucket } from "../types";
import { formatDateTime, formatDurationLong } from "./activity";

interface BuildDeviceUsageTreemapOptions {
  topBuckets?: number;
}

export function buildDeviceUsageTreemap(
  analysis: DeviceAnalysisResponse | null,
  { topBuckets = 14 }: BuildDeviceUsageTreemapOptions = {}
): TreemapNodeInput[] {
  if (!analysis) {
    return [];
  }

  const accent = "#4ea57d";
  const buckets = [...analysis.appUsage].sort((left, right) => right.totalTrackedMs - left.totalTrackedMs);
  if (buckets.length === 0) {
    return [];
  }

  const visibleBuckets = buckets.slice(0, topBuckets);
  const hiddenBuckets = buckets.slice(topBuckets);
  const maxTrackedMs = visibleBuckets[0]?.totalTrackedMs ?? 0;
  const totalTrackedMs = analysis.totalTrackedMs;

  const items = visibleBuckets.map((bucket, index) =>
    buildUsageLeaf(`app-${index}`, bucket, totalTrackedMs, accent, maxTrackedMs)
  );

  if (hiddenBuckets.length > 0) {
    const mergedTrackedMs = hiddenBuckets.reduce((sum, bucket) => sum + bucket.totalTrackedMs, 0);
    const mergedSessions = hiddenBuckets.reduce((sum, bucket) => sum + bucket.sessions, 0);

    items.push({
      id: "app-others",
      label: `其他 ${hiddenBuckets.length} 项`,
      value: mergedTrackedMs,
      colorValue: usageRatio(totalTrackedMs, mergedTrackedMs),
      meta: `${mergedSessions} 次记录`,
      note: `已合并较小分组 · ${formatDurationLong(mergedTrackedMs)}`,
      accent: tintAccent(accent, 0.34)
    });
  }

  return items;
}

function buildUsageLeaf(
  id: string,
  bucket: UsageBucket,
  totalTrackedMs: number,
  accent: string,
  maxTrackedMs: number
): TreemapNodeInput {
  const normalized = maxTrackedMs > 0 ? bucket.totalTrackedMs / maxTrackedMs : 0;

  return {
    id,
    label: bucket.label,
    value: bucket.totalTrackedMs,
    colorValue: usageRatio(totalTrackedMs, bucket.totalTrackedMs),
    meta: bucket.sublabel || `${bucket.sessions} 次记录`,
    note: `${bucket.sessions} 次记录 · 最近 ${formatDateTime(bucket.lastSeen)}`,
    accent: tintAccent(accent, 0.2 + normalized * 0.65)
  };
}

function usageRatio(totalTrackedMs: number, bucketTrackedMs: number): number {
  if (totalTrackedMs <= 0) {
    return 0;
  }

  return (bucketTrackedMs / totalTrackedMs) * 100;
}

function tintAccent(hexColor: string, amount: number): string {
  const safeAmount = Math.max(0, Math.min(amount, 1));
  const hex = hexColor.replace("#", "");
  const normalized = hex.length === 3
    ? hex
        .split("")
        .map((channel) => channel + channel)
        .join("")
    : hex;

  if (normalized.length !== 6) {
    return hexColor;
  }

  const source = Number.parseInt(normalized, 16);
  const target = 0xffffff;

  const r = Math.round(((source >> 16) & 255) + ((((target >> 16) & 255) - ((source >> 16) & 255)) * safeAmount));
  const g = Math.round(((source >> 8) & 255) + ((((target >> 8) & 255) - ((source >> 8) & 255)) * safeAmount));
  const b = Math.round((source & 255) + (((target & 255) - (source & 255)) * safeAmount));

  return `#${[r, g, b].map((channel) => channel.toString(16).padStart(2, "0")).join("")}`;
}
