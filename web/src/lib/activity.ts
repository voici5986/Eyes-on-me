import type { ActivityEvent, DashboardSnapshot, DeviceStatus, PresenceState } from "../types";

function presenceLabel(presence: PresenceState): string | null {
  switch (presence) {
    case "locked":
      return "屏幕已锁定";
    case "idle":
      return "当前空闲中";
    default:
      return null;
  }
}

export function formatTime(value: string | undefined | null): string {
  if (!value) {
    return "n/a";
  }

  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit"
  }).format(new Date(value));
}

export function formatDateTime(value: string | undefined | null): string {
  if (!value) {
    return "n/a";
  }

  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(value));
}

export function activityHeadline(activity: ActivityEvent): string {
  const label = presenceLabel(activity.presence);
  if (label) {
    return label;
  }

  return activity.browser?.pageTitle || activity.windowTitle || activity.app.title || activity.app.name;
}

export function activitySubline(activity: ActivityEvent): string {
  if (activity.presence === "locked") {
    return "锁屏或熄屏期间不计入活跃时长";
  }

  if (activity.presence === "idle") {
    return "检测到离开设备，已暂停活跃统计";
  }

  return activity.browser?.domain || activity.browser?.url || activity.app.id;
}

export function activityUrl(activity: ActivityEvent): string | null {
  return activity.browser?.url || null;
}

export function activityDurationMs(activity: ActivityEvent, nowMs: number): number {
  return Math.max(0, nowMs - new Date(activity.ts).getTime());
}

export function isFreshActivity(activity: ActivityEvent, nowMs: number, maxAgeMs = 35_000): boolean {
  return activityDurationMs(activity, nowMs) <= maxAgeMs;
}

export function recentActivityDurationMs(
  recentActivities: ActivityEvent[],
  activity: ActivityEvent,
  index: number,
  nowMs: number
): number {
  const currentTs = new Date(activity.ts).getTime();
  const nextForDevice = recentActivities.find(
    (candidate, candidateIndex) => candidateIndex < index && candidate.deviceId === activity.deviceId
  );

  if (!nextForDevice) {
    return Math.max(0, nowMs - currentTs);
  }

  return Math.max(0, new Date(nextForDevice.ts).getTime() - currentTs);
}

export function formatDuration(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }

  if (minutes > 0) {
    return `${minutes}m ${seconds}s`;
  }

  return `${seconds}s`;
}

export function formatDurationLong(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return `${hours}h ${minutes}m ${seconds}s`;
  }

  if (minutes > 0) {
    return `${minutes}m ${seconds}s`;
  }

  return `${seconds}s`;
}

export function usageShare(totalMs: number, partMs: number): number {
  if (totalMs <= 0) {
    return 0;
  }

  return Math.min(100, Math.max(0, (partMs / totalMs) * 100));
}

export function deriveStatusFromActivity(activity: ActivityEvent | null): string {
  if (!activity) {
    return "暂无状态";
  }

  const label = presenceLabel(activity.presence);
  if (label) {
    return label;
  }

  if (activity.browser?.domain) {
    return `正在使用 ${activity.app.name} 浏览 ${activity.browser.domain}`;
  }

  if (activity.browser?.pageTitle) {
    return `正在使用 ${activity.app.name} 查看 ${activity.browser.pageTitle}`;
  }

  if (activity.windowTitle) {
    return `正在使用 ${activity.app.name} · ${activity.windowTitle}`;
  }

  return `正在使用 ${activity.app.name}`;
}

export function latestStatusForDevice(snapshot: DashboardSnapshot | null, deviceId: string): DeviceStatus | null {
  if (!snapshot) {
    return null;
  }

  if (snapshot.latestStatus?.deviceId === deviceId) {
    return snapshot.latestStatus;
  }

  return null;
}
