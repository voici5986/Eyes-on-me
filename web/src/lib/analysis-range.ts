import type { AnalysisRange } from "../types";

export const DEFAULT_ANALYSIS_RANGE: AnalysisRange = "today";

export const ANALYSIS_RANGE_OPTIONS: Array<{
  value: AnalysisRange;
  label: string;
  description: string;
}> = [
  { value: "3h", label: "3 小时", description: "最近 3 小时" },
  { value: "6h", label: "6 小时", description: "最近 6 小时" },
  { value: "1d", label: "1 天", description: "最近 24 小时" },
  { value: "1w", label: "1 周", description: "最近 7 天" },
  { value: "1m", label: "1 月", description: "最近 30 天" },
  { value: "all", label: "全部", description: "全部历史记录" },
  { value: "today", label: "今天", description: "今天" }
];

export function normalizeAnalysisRange(value: unknown): AnalysisRange {
  return ANALYSIS_RANGE_OPTIONS.find((item) => item.value === value)?.value ?? DEFAULT_ANALYSIS_RANGE;
}

export function analysisRangeLabel(range: AnalysisRange): string {
  return ANALYSIS_RANGE_OPTIONS.find((item) => item.value === range)?.description ?? "今天";
}
